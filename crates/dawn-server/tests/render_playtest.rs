//! Exercise the deployed binary with a cloud-style PORT and disposable invites.

use dawn_core::{ClientRequest, JumpGateId, ResumeTicket};
use dawn_protocol::{ClientMessage, HelloMessage, InitialStateWire, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout, Instant};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Error, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Server {
    child: Child,
    _directory: tempfile::TempDir,
    url: String,
    log: std::path::PathBuf,
}

impl Server {
    fn spawn(invited: bool) -> Self {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_some_and(|ext| ext == "toml") {
                std::fs::copy(entry.path(), data.join(entry.file_name())).unwrap();
            }
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_simulate"));
        let log = directory.path().join("server.log");
        let output = std::fs::File::create(&log).unwrap();
        command
            .args(["--serve", "--cluster", "--ships", "0", "--pop-cap", "8"])
            .current_dir(directory.path())
            .env("PORT", port.to_string())
            .env_remove("DAWN_INVITE_TOKENS")
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output));
        if invited {
            command.env("DAWN_INVITE_TOKENS", format!("[\"{}\"]", "a".repeat(64)));
        }
        drop(reservation);
        let child = command.spawn().unwrap();
        Self {
            child,
            _directory: directory,
            url: format!("ws://127.0.0.1:{port}"),
            log,
        }
    }

    async fn connect(&mut self, token: Option<&str>) -> Result<Socket, Error> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let mut request = self.url.clone().into_client_request().unwrap();
            if let Some(token) = token {
                request
                    .headers_mut()
                    .insert("authorization", format!("Bearer {token}").parse().unwrap());
            }
            match tokio_tungstenite::connect_async(request).await {
                Ok((socket, _)) => return Ok(socket),
                Err(Error::Io(_)) if Instant::now() < deadline => {
                    assert!(
                        self.child.try_wait().unwrap().is_none(),
                        "server exited: {}",
                        self.logs()
                    );
                    sleep(Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn logs(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn send(socket: &mut Socket, message: ClientMessage) {
    socket
        .send(Message::Binary(message.encode().unwrap().into()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Socket) -> ServerMessage {
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Binary(bytes) => return ServerMessage::decode(&bytes).unwrap(),
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
            other => panic!("unexpected WebSocket frame: {other:?}"),
        }
    }
}

async fn welcome(
    socket: &mut Socket,
    resume: Option<ResumeTicket>,
) -> (u64, u64, ResumeTicket, InitialStateWire) {
    send(socket, ClientMessage::Hello(HelloMessage { resume })).await;
    let ServerMessage::Welcome {
        player_id,
        ship_id,
        resume_ticket,
    } = receive(socket).await
    else {
        panic!("expected Welcome");
    };
    loop {
        if let ServerMessage::InitialState(state) = receive(socket).await {
            return (player_id, ship_id, resume_ticket, state);
        }
    }
}

#[tokio::test]
async fn cloud_cluster_rejects_missing_invites_on_startup() {
    let mut server = Server::spawn(false);
    timeout(Duration::from_secs(8), async {
        loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                assert!(!status.success());
                assert!(
                    server.logs().contains("DAWN_INVITE_TOKENS is required"),
                    "{}",
                    server.logs()
                );
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("cloud cluster must fail closed without invites");
}

#[tokio::test]
async fn invited_cloud_cluster_jumps_and_resumes_on_the_same_endpoint() {
    let mut server = Server::spawn(true);
    let result = timeout(Duration::from_secs(60), async {
        let Err(Error::Http(response)) = server.connect(None).await else {
            panic!("cloud endpoint must reject missing invite before Hello");
        };
        assert_eq!(response.status().as_u16(), 401);
        let Err(Error::Http(response)) = server.connect(Some(&"c".repeat(64))).await else {
            panic!("cloud endpoint must reject invalid invite before Hello");
        };
        assert_eq!(response.status().as_u16(), 401);
        let mut socket = server.connect(Some(&"a".repeat(64))).await.unwrap();
        let (player, ship, ticket, initial) = welcome(&mut socket, None).await;
        let gate = initial.jump_gates.first().expect("local gate");
        let destination = gate.to_system_name.clone();
        assert_ne!(initial.system_name, destination);
        send(
            &mut socket,
            ClientMessage::Command(ClientRequest::Jump {
                gate: JumpGateId(gate.gate_id),
            }),
        )
        .await;
        loop {
            match receive(&mut socket).await {
                ServerMessage::InitialState(state) if state.system_name == destination => break,
                ServerMessage::ClientRequestRejected(error) => panic!("jump rejected: {error:?}"),
                ServerMessage::Redirect { .. } => {
                    panic!("in-process transit must keep one endpoint")
                }
                _ => {}
            }
        }
        socket.close(None).await.unwrap();
        let mut resumed = server.connect(Some(&"a".repeat(64))).await.unwrap();
        let (resumed_player, resumed_ship, _, state) = welcome(&mut resumed, Some(ticket)).await;
        assert_eq!((resumed_player, resumed_ship), (player, ship));
        assert_eq!(state.system_name, destination);
    })
    .await;
    assert!(
        result.is_ok(),
        "invited cloud gate transit and resume timed out: {}",
        server.logs()
    );
}
