use crate::inbound_delivery;
use crate::loadout_gd::PlayerLoadout;
use crate::server_message_validation::decode_server_message;
use crate::world_session_gd::WorldSession;
use dawn_protocol::ServerMessage;
use godot::prelude::*;

#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ServerMessageDecoder {}

#[godot_api]
impl ServerMessageDecoder {
    #[func]
    fn decode(&self, bytes: PackedByteArray) -> Option<Gd<ServerMessageOutcome>> {
        match decode_server_message(bytes.as_slice()) {
            Ok(message) => Some(Gd::from_object(ServerMessageOutcome { message })),
            Err(error) => {
                godot_error!("ServerMessageDecoder.decode: {error}");
                None
            }
        }
    }

    #[cfg(debug_assertions)]
    #[func]
    fn test_outcome(&self, kind: GString) -> Option<Gd<ServerMessageOutcome>> {
        let message = crate::server_message_fixture::message(kind.to_string().as_str())?;
        self.decode(PackedByteArray::from(
            message
                .encode()
                .expect("typed server message must encode")
                .as_slice(),
        ))
    }
}

/// Validated server message and the receive path's sole Godot dispatch seam.
#[derive(GodotClass)]
#[class(no_init)]
pub struct ServerMessageOutcome {
    message: ServerMessage,
}

#[godot_api]
impl ServerMessageOutcome {
    #[func]
    fn dispatch(
        &self,
        connection_target: Gd<Object>,
        world_target: Gd<Object>,
        session: Gd<WorldSession>,
        loadout: Gd<PlayerLoadout>,
        connection_ship_id: i64,
    ) -> bool {
        inbound_delivery::dispatch(
            &self.message,
            connection_target,
            world_target,
            session,
            loadout,
            connection_ship_id,
        )
    }
}
