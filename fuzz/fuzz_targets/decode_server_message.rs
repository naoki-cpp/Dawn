#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(message) = dawn_wire::ServerMessage::decode(data) {
        let Ok(encoded) = message.encode() else {
            return;
        };
        assert!(!encoded.is_empty(), "a decoded ServerMessage must re-encode");
        assert!(
            dawn_wire::ServerMessage::decode(&encoded).is_ok(),
            "a decoded ServerMessage must round-trip through postcard"
        );
    }
});
