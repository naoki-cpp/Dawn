#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(message) = dawn_wire::ServerMessage::decode(data) {
        let encoded = message.encode();
        assert!(!encoded.is_empty(), "a decoded ServerMessage must re-encode");
        assert!(
            dawn_wire::ServerMessage::decode(&encoded).is_ok(),
            "a decoded ServerMessage must round-trip through postcard"
        );
    }
});
