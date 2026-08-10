#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(message) = dawn_wire::ClientMessage::decode(data) {
        let Ok(encoded) = message.encode() else {
            return;
        };
        assert!(!encoded.is_empty(), "a decoded ClientMessage must re-encode");
        assert!(
            dawn_wire::ClientMessage::decode(&encoded).is_ok(),
            "a decoded ClientMessage must round-trip through postcard"
        );
    }
});
