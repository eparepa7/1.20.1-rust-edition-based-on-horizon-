//! Isolated 1.20.1 login packet codecs. This is not a complete protocol adapter:
//! the game must not advertise support for protocol 763 on the basis of these.
use crate::{
    buf::{Bytes, Codec, Ident, List, Opt, Reader, Str, StrN, Uuid, VarInt, Writer},
    connection::Connection,
    packet, packet_enum,
    packets::{handshake, login},
};

/// Minecraft Java 1.20.1 protocol; this module alone does not provide game support.
pub const PROTOCOL: i32 = 763;
const MAX_QUERY_BYTES: usize = 1_048_576;

/// A plugin query's remaining bytes, limited as in the 1.20.1 packet reader.
pub struct QueryData;
impl Codec<Vec<u8>> for QueryData {
    fn read(reader: &mut Reader) -> crate::Result<Vec<u8>> {
        if reader.remaining() > MAX_QUERY_BYTES {
            return Err(crate::Error::new("login query payload exceeds 1 MiB"));
        }
        Ok(reader.rest().to_vec())
    }
    fn write(value: &Vec<u8>, writer: &mut Writer) {
        writer.bytes(value);
    }
}

mod serverbound_ids {
    pub const HELLO: i32 = 0;
    pub const KEY: i32 = 1;
    pub const QUERY_ANSWER: i32 = 2;
}
mod clientbound_ids {
    pub const DISCONNECT: i32 = 0;
    pub const HELLO: i32 = 1;
    pub const FINISHED: i32 = 2;
    pub const COMPRESSION: i32 = 3;
    pub const QUERY: i32 = 4;
}

packet!(
    /// The 1.20.1 login hello carries an optional profile UUID.
    ServerHello { name: String => StrN<16>, uuid: Option<u128> => Opt<Uuid> }
);
packet!(
    /// The 1.20.1 encryption request has no modern authentication flag.
    EncryptionRequest {
        server_id: String => StrN<20>,
        public_key: Vec<u8> => Bytes,
        challenge: Vec<u8> => Bytes,
    }
);
packet!(
    /// The 1.20.1 property fields use the old general UTF-8 string limits.
    Property {
        name: String => Str,
        value: String => Str,
        signature: Option<String> => Opt<Str>,
    }
);
packet!(
    /// The profile sent on successful login, before entering play directly.
    GameProfile {
        uuid: u128 => Uuid,
        name: String => StrN<16>,
        properties: Vec<Property> => List<Property>,
    }
);
packet!(LoginFinished { profile: GameProfile => GameProfile });
packet!(
    CustomQuery {
        transaction: i32 => VarInt,
        channel: String => Ident,
        data: Vec<u8> => QueryData,
    }
);
packet!(CustomQueryAnswer { transaction: i32 => VarInt, payload: Option<Vec<u8>> => Opt<QueryData> });

packet_enum!(Serverbound, crate::packets::legacy_1_20_1::serverbound_ids {
    Hello(ServerHello) = HELLO,
    Key(login::Key) = KEY,
    CustomQueryAnswer(CustomQueryAnswer) = QUERY_ANSWER,
});
packet_enum!(Clientbound, crate::packets::legacy_1_20_1::clientbound_ids {
    Disconnect(login::Disconnect) = DISCONNECT,
    Hello(EncryptionRequest) = HELLO,
    LoginFinished(LoginFinished) = FINISHED,
    Compression(login::Compression) = COMPRESSION,
    CustomQuery(CustomQuery) = QUERY,
});

/// Perform only the 1.20.1 offline *login* on a fresh, unadapted connection.
/// The caller must set a read timeout. On success the connection has entered
/// 1.20.1 play state; game packets and the rest of the engine are not ported.
/// An encryption request is an error here: this function never bypasses an
/// online-mode server's authentication.
pub fn offline_login(
    conn: &mut Connection,
    host: &str,
    port: u16,
    name: &str,
    uuid: Option<u128>,
) -> crate::Result<GameProfile> {
    if host.encode_utf16().count() > 255 || name.is_empty() || name.encode_utf16().count() > 16 {
        return Err(crate::Error::new("invalid 1.20.1 host or player name"));
    }
    let intention = handshake::Serverbound::Intention(handshake::Intention {
        protocol: PROTOCOL,
        host: host.into(),
        port,
        intent: handshake::LOGIN,
    });
    conn.send(&intention.encode())?;
    conn.send(
        &Serverbound::Hello(ServerHello {
            name: name.into(),
            uuid,
        })
        .encode(),
    )?;
    loop {
        let (id, payload) = conn.recv()?;
        match Clientbound::decode(id, &payload)? {
            Clientbound::Compression(packet) => conn.set_compression(packet.threshold),
            Clientbound::CustomQuery(query) => {
                // Vanilla declines unknown login plugin channels with an absent response.
                let response = Serverbound::CustomQueryAnswer(CustomQueryAnswer {
                    transaction: query.transaction,
                    payload: None,
                });
                conn.send(&response.encode())?;
            }
            Clientbound::LoginFinished(packet) => return Ok(packet.profile),
            Clientbound::Disconnect(packet) => {
                return Err(crate::Error::new(format!(
                    "server disconnected: {}",
                    packet.reason_json
                )));
            }
            Clientbound::Hello(_) => {
                return Err(crate::Error::new(
                    "server requested authentication; offline login is unavailable",
                ));
            }
            Clientbound::Unknown(id, _) => {
                return Err(crate::Error::new(format!(
                    "unknown 1.20.1 login packet {id}"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Pipe;
    use std::{thread, time::Duration};

    #[test]
    fn login_hello_matches_original_1_20_1_serialization() {
        // Payloads measured independently using the original 1.20.1 server.
        let offline = Serverbound::Hello(ServerHello {
            name: "Alex".into(),
            uuid: None,
        });
        let expected = b"\x00\x04Alex\x00";
        assert_eq!(offline.encode(), expected);
        assert_eq!(Serverbound::decode(0, &expected[1..]).unwrap(), offline);

        let with_uuid = Serverbound::Hello(ServerHello {
            name: "Developer".into(),
            uuid: Some(0x00112233445566778899aabbccddeeff),
        });
        let expected = b"\x00\x09Developer\x01\x00\x11\x22\x33\x44\x55\x66\x77\x88\x99\xaa\xbb\xcc\xdd\xee\xff";
        assert_eq!(with_uuid.encode(), expected);
        assert_eq!(Serverbound::decode(0, &expected[1..]).unwrap(), with_uuid);
    }

    #[test]
    fn encryption_request_matches_original_1_20_1_serialization() {
        let packet = Clientbound::Hello(EncryptionRequest {
            server_id: String::new(),
            public_key: vec![1, 2, 3],
            challenge: vec![4, 5],
        });
        let expected = [1, 0, 3, 1, 2, 3, 2, 4, 5];
        assert_eq!(packet.encode(), expected);
        assert_eq!(Clientbound::decode(1, &expected[1..]).unwrap(), packet);
        assert!(Clientbound::decode(1, &expected[1..expected.len() - 1]).is_err());
        let mut extra = expected.to_vec();
        extra.push(0);
        assert!(Clientbound::decode(1, &extra[1..]).is_err());
    }

    #[test]
    fn login_packets_match_original_1_20_1_serialization() {
        // Expected payloads from the original 1.20.1 server's packet writers.
        let profile = Clientbound::LoginFinished(LoginFinished {
            profile: GameProfile {
                uuid: 0x00112233445566778899aabbccddeeff,
                name: "Alex".into(),
                properties: vec![Property {
                    name: "textures".into(),
                    value: "abc".into(),
                    signature: Some("sig".into()),
                }],
            },
        });
        let profile_bytes = b"\x02\x00\x11\x22\x33\x44\x55\x66\x77\x88\x99\xaa\xbb\xcc\xdd\xee\xff\x04Alex\x01\x08textures\x03abc\x01\x03sig";
        assert_eq!(profile.encode(), profile_bytes);
        assert_eq!(
            Clientbound::decode(2, &profile_bytes[1..]).unwrap(),
            profile
        );

        let compression = Clientbound::Compression(login::Compression { threshold: 256 });
        assert_eq!(compression.encode(), [3, 0x80, 2]);
        assert_eq!(Clientbound::decode(3, &[0x80, 2]).unwrap(), compression);
        let disconnect = Clientbound::Disconnect(login::Disconnect {
            reason_json: "{\"text\":\"bye\"}".into(),
        });
        assert_eq!(disconnect.encode(), b"\x00\x0e{\"text\":\"bye\"}");

        let query = Clientbound::CustomQuery(CustomQuery {
            transaction: 42,
            channel: "example:probe".into(),
            data: vec![7, 8],
        });
        let query_bytes = b"\x04\x2a\x0dexample:probe\x07\x08";
        assert_eq!(query.encode(), query_bytes);
        assert_eq!(Clientbound::decode(4, &query_bytes[1..]).unwrap(), query);
        let response = Serverbound::CustomQueryAnswer(CustomQueryAnswer {
            transaction: 42,
            payload: None,
        });
        assert_eq!(response.encode(), [2, 42, 0]);
        assert_eq!(Serverbound::decode(2, &[42, 0]).unwrap(), response);
        let response_with_data = Serverbound::CustomQueryAnswer(CustomQueryAnswer {
            transaction: 42,
            payload: Some(vec![1, 2, 3]),
        });
        assert_eq!(response_with_data.encode(), [2, 42, 1, 1, 2, 3]);
        assert_eq!(
            Serverbound::decode(2, &[42, 1, 1, 2, 3]).unwrap(),
            response_with_data
        );
    }

    #[test]
    fn offline_login_changes_compression_and_enters_play_without_acknowledgement() {
        let (client_pipe, server_pipe) = Pipe::pair();
        let mut client = Connection::local(client_pipe);
        let mut server = Connection::local(server_pipe);
        client
            .reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        server
            .reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let fake_server = thread::spawn(move || {
            let (id, payload) = server.recv().unwrap();
            assert_eq!(
                (id, payload.as_slice()),
                (0, &b"\xfb\x05\x09localhost\x63\xdd\x02"[..])
            );
            let (id, payload) = server.recv().unwrap();
            assert_eq!((id, payload.as_slice()), (0, &b"\x04Alex\x00"[..]));
            server.send(&[3, 0x80, 2]).unwrap();
            server.set_compression(256);
            server.send(b"\x04\x2a\x0dexample:probe\x07\x08").unwrap();
            let (id, payload) = server.recv().unwrap();
            assert_eq!((id, payload.as_slice()), (2, &[42, 0][..]));
            server.send(b"\x02\x00\x11\x22\x33\x44\x55\x66\x77\x88\x99\xaa\xbb\xcc\xdd\xee\xff\x04Alex\x01\x08textures\x03abc\x01\x03sig").unwrap();
            // 1.20.1 must not send the newer LoginAcknowledged packet.
            server
                .reader
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            assert!(server.recv().is_err());
        });
        let profile = offline_login(&mut client, "localhost", 25565, "Alex", None).unwrap();
        assert_eq!(profile.name, "Alex");
        assert_eq!(profile.uuid, 0x00112233445566778899aabbccddeeff);
        assert_eq!(profile.properties[0].signature.as_deref(), Some("sig"));
        fake_server.join().unwrap();
    }

    #[test]
    fn offline_login_rejects_an_authentication_request() {
        let (client_pipe, server_pipe) = Pipe::pair();
        let mut client = Connection::local(client_pipe);
        let mut server = Connection::local(server_pipe);
        client
            .reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        server
            .reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let fake_server = thread::spawn(move || {
            server.recv().unwrap();
            server.recv().unwrap();
            server.send(&[1, 0, 3, 1, 2, 3, 2, 4, 5]).unwrap();
        });
        let error = offline_login(&mut client, "localhost", 25565, "Alex", None).unwrap_err();
        assert!(error.to_string().contains("authentication"));
        fake_server.join().unwrap();
    }

    #[test]
    fn malformed_queries_are_rejected_and_763_is_not_advertised() {
        let mut query = b"\x2a\x0dexample:probe".to_vec();
        query.resize(query.len() + MAX_QUERY_BYTES + 1, 0);
        assert!(Clientbound::decode(4, &query).is_err());
        assert_eq!(crate::adapter::ServerVersion::from_protocol(PROTOCOL), None);
    }
}
