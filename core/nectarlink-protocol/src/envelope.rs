// SPDX-License-Identifier: MPL-2.0
//! The message envelope carried in every frame (protocol §3.1).

use ciborium::Value;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    ErrorCode, ProtocolError,
    messages::{ErrorBody, types},
};

/// `{ t: type, id?: request id, re?: reply-to id, b?: body }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub t: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub re: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<Value>,
}

impl Envelope {
    /// A message with a typed body.
    pub fn new<T: Serialize>(t: &str, body: &T) -> Result<Self, ProtocolError> {
        let b = Value::serialized(body).map_err(|e| ProtocolError::BadMessage(e.to_string()))?;
        Ok(Envelope { t: t.to_owned(), id: None, re: None, b: Some(b) })
    }

    /// A message without a body.
    pub fn empty(t: &str) -> Self {
        Envelope { t: t.to_owned(), id: None, re: None, b: None }
    }

    /// An `error` reply.
    pub fn error(code: ErrorCode, msg: impl Into<String>) -> Self {
        let body = ErrorBody { code, msg: Some(msg.into()) };
        Envelope::new(types::ERROR, &body).expect("error bodies always serialize")
    }

    pub fn with_id(mut self, id: u64) -> Self {
        self.id = Some(id);
        self
    }

    pub fn reply_to(mut self, id: Option<u64>) -> Self {
        self.re = id;
        self
    }

    /// Decodes the body. A missing body decodes as an empty map, so messages
    /// whose fields are all optional work without one.
    pub fn body<T: DeserializeOwned>(&self) -> Result<T, ProtocolError> {
        let value = self.b.clone().unwrap_or(Value::Map(Vec::new()));
        value.deserialized().map_err(|e| ProtocolError::BadMessage(format!("{}: {e}", self.t)))
    }

    /// Checks the message type, turning a peer `error` into
    /// [`ProtocolError::Remote`].
    pub fn expect(&self, expected: &'static str) -> Result<(), ProtocolError> {
        if self.t == expected {
            return Ok(());
        }
        if self.t == types::ERROR {
            let err: ErrorBody = self.body()?;
            return Err(ProtocolError::Remote { code: err.code, msg: err.msg.unwrap_or_default() });
        }
        Err(ProtocolError::UnexpectedType { expected, got: self.t.clone() })
    }

    /// Like [`expect`](Self::expect), then decodes the body.
    pub fn expect_body<T: DeserializeOwned>(&self, expected: &'static str) -> Result<T, ProtocolError> {
        self.expect(expected)?;
        self.body()
    }

    pub fn to_cbor(&self) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).expect("writing to a Vec cannot fail");
        out
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, ProtocolError> {
        ciborium::from_reader(bytes).map_err(|e| ProtocolError::BadMessage(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{Ping, types};

    #[test]
    fn round_trips_through_cbor() {
        let env = Envelope::new(types::PING, &Ping { ts: 42 }).unwrap().with_id(7);
        let decoded = Envelope::from_cbor(&env.to_cbor()).unwrap();
        assert_eq!(decoded, env);
        assert_eq!(decoded.expect_body::<Ping>(types::PING).unwrap().ts, 42);
    }

    #[test]
    fn ignores_unknown_fields() {
        // A future peer adds an envelope key and a body field.
        let raw = Value::Map(vec![
            (Value::Text("t".into()), Value::Text("ping".into())),
            (Value::Text("future".into()), Value::Bool(true)),
            (
                Value::Text("b".into()),
                Value::Map(vec![
                    (Value::Text("ts".into()), Value::Integer(5.into())),
                    (Value::Text("extra".into()), Value::Text("ignored".into())),
                ]),
            ),
        ]);
        let mut bytes = Vec::new();
        ciborium::into_writer(&raw, &mut bytes).unwrap();
        let env = Envelope::from_cbor(&bytes).unwrap();
        assert_eq!(env.body::<Ping>().unwrap().ts, 5);
    }

    #[test]
    fn remote_errors_surface_through_expect() {
        let env = Envelope::error(ErrorCode::Denied, "nope").reply_to(Some(1));
        match env.expect(types::OK) {
            Err(ProtocolError::Remote { code, msg }) => {
                assert_eq!(code, ErrorCode::Denied);
                assert_eq!(msg, "nope");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn garbage_is_a_bad_message() {
        assert!(matches!(Envelope::from_cbor(&[0xff, 0x00, 0x13]), Err(ProtocolError::BadMessage(_))));
    }
}
