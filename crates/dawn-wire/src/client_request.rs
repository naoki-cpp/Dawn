use crate::ClientRequestEnvelope;
#[cfg(test)]
use dawn_core::ClientRequest;

/// Render the client -> server request schema from the single protocol authority.
pub fn client_request_json_schema() -> schemars::Schema {
    schemars::schema_for!(ClientRequestEnvelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{EntityId, NodeId, Position, ShipId};

    #[test]
    fn typed_request_round_trips_through_postcard() {
        let request = ClientRequest::Move {
            target: Position::new(10.0, 0.0, -5.0),
        };
        let bytes = postcard::to_stdvec(&request).expect("encode ClientRequest");
        let decoded: ClientRequest = postcard::from_bytes(&bytes).expect("decode ClientRequest");
        assert_eq!(decoded, request);
    }

    #[test]
    fn active_ship_identity_is_not_part_of_lock_on_request() {
        let target = ShipId(EntityId::new(NodeId(0), 7));
        let request = ClientRequest::LockOn { target };
        assert!(matches!(request, ClientRequest::LockOn { target: actual } if actual == target));
    }

    #[test]
    fn schema_is_generated_from_client_request() {
        let schema = client_request_json_schema();
        let json = serde_json::to_value(schema).expect("schema JSON");
        assert!(json.to_string().contains("ClientRequest"));
    }
}
