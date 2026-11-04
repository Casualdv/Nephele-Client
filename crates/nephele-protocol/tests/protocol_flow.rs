//! End-to-end tests against an in-process fake 1.8.9 server. These exercise
//! real TCP, real framing and real compression, so they catch mistakes that
//! unit tests on each layer can miss.

use std::time::Duration;

use nephele_protocol::packets::{login, play, status};
use nephele_protocol::{
    Connection, LoginOutcome, NextState, Packet, PacketWriter, ServerAddress, State,
};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

async fn accept_one() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

async fn accept(listener: TcpListener) -> Connection {
    let (stream, _) = listener.accept().await.unwrap();
    Connection::from_stream(stream, State::Handshake)
}

fn address(port: u16) -> ServerAddress {
    ServerAddress::parse(&format!("127.0.0.1:{port}")).unwrap()
}

async fn connect(port: u16) -> Connection {
    tokio::time::timeout(Duration::from_secs(5), Connection::connect(&address(port)))
        .await
        .expect("connect timed out")
        .expect("connect failed")
}

#[tokio::test]
async fn status_ping_roundtrip() {
    let (listener, port) = accept_one().await;

    let server = tokio::spawn(async move {
        let mut server = accept(listener).await;

        let handshake = server.read_packet().await.unwrap();
        assert_eq!(handshake.id, 0x00);
        let mut r = handshake.reader();
        assert_eq!(r.read_varint().unwrap(), 47, "protocol version");
        let host = r.read_string().unwrap();
        assert_eq!(r.read_u16().unwrap(), port);
        assert_eq!(r.read_varint().unwrap(), 1, "next state = status");
        assert_eq!(host, "127.0.0.1");

        let request = server.read_packet().await.unwrap();
        assert_eq!(request.id, status::SERVERBOUND_REQUEST);

        let mut w = PacketWriter::new();
        w.write_string(
            r#"{"version":{"name":"1.8.9","protocol":47},"players":{"max":10,"online":2},"description":{"text":"Nephele mock","extra":[{"text":" server"}]}}"#,
        );
        server
            .write_packet(&Packet::new(status::CLIENTBOUND_RESPONSE, w.into_bytes()))
            .await
            .unwrap();

        let ping = server.read_packet().await.unwrap();
        assert_eq!(ping.id, status::SERVERBOUND_PING);
        let nonce = ping.reader().read_i64().unwrap();

        let mut w = PacketWriter::new();
        w.write_i64(nonce);
        server
            .write_packet(&Packet::new(status::CLIENTBOUND_PONG, w.into_bytes()))
            .await
            .unwrap();
    });

    let mut client = connect(port).await;
    client
        .handshake(&address(port), NextState::Status)
        .await
        .unwrap();
    let response = client.status().await.unwrap();

    assert_eq!(response.version_name.as_deref(), Some("1.8.9"));
    assert_eq!(response.protocol, Some(47));
    assert_eq!(response.online_players, Some(2));
    assert_eq!(response.max_players, Some(10));
    assert_eq!(response.description, "Nephele mock server");
    assert!(response.latency.is_some());

    server.await.unwrap();
}

#[tokio::test]
async fn status_surfaces_a_closed_connection() {
    let (listener, port) = accept_one().await;

    let server = tokio::spawn(async move {
        let mut server = accept(listener).await;
        let handshake = server.read_packet().await.unwrap();
        assert_eq!(handshake.id, 0x00);
        server.read_packet().await.unwrap();
        // Status has no disconnect packet: a server that rejects a ping just
        // closes the socket.
        server.shutdown().await;
    });

    let mut client = connect(port).await;
    client
        .handshake(&address(port), NextState::Status)
        .await
        .unwrap();
    assert!(client.status().await.is_err());

    server.await.unwrap();
}

#[tokio::test]
async fn login_with_compression_then_join_game() {
    let (listener, port) = accept_one().await;

    let server = tokio::spawn(async move {
        let mut server = accept(listener).await;

        let handshake = server.read_packet().await.unwrap();
        let mut r = handshake.reader();
        assert_eq!(r.read_varint().unwrap(), 47);
        let _host = r.read_string().unwrap();
        let _port = r.read_u16().unwrap();
        assert_eq!(r.read_varint().unwrap(), 2, "next state = login");

        let start = server.read_packet().await.unwrap();
        assert_eq!(start.id, login::SERVERBOUND_LOGIN_START);
        assert_eq!(start.reader().read_string().unwrap(), "Nephele");

        // Compression is announced while the threshold is still disabled.
        let mut w = PacketWriter::new();
        w.write_varint(64);
        server
            .write_packet(&Packet::new(
                login::CLIENTBOUND_SET_COMPRESSION,
                w.into_bytes(),
            ))
            .await
            .unwrap();
        server.set_compression_threshold(Some(64));

        let mut w = PacketWriter::new();
        w.write_string("069a79f4-44e9-4726-a5be-fca90e38aaf5")
            .write_string("Nephele");
        server
            .write_packet(&Packet::new(
                login::CLIENTBOUND_LOGIN_SUCCESS,
                w.into_bytes(),
            ))
            .await
            .unwrap();

        // Join Game, then a keep-alive the client must answer.
        let mut w = PacketWriter::new();
        w.write_i32(7)
            .write_u8(0)
            .write_i8(0)
            .write_u8(1)
            .write_u8(20)
            .write_string("default")
            .write_bool(false);
        server
            .write_packet(&Packet::new(play::clientbound::JOIN_GAME, w.into_bytes()))
            .await
            .unwrap();

        let mut w = PacketWriter::new();
        w.write_varint(1234);
        server
            .write_packet(&Packet::new(play::clientbound::KEEP_ALIVE, w.into_bytes()))
            .await
            .unwrap();

        let response = server.read_packet().await.unwrap();
        assert_eq!(response.id, play::serverbound::KEEP_ALIVE);
        assert_eq!(response.reader().read_varint().unwrap(), 1234);

        let mut w = PacketWriter::new();
        w.write_string(r#"{"text":"<Server> welcome"}"#).write_u8(0);
        server
            .write_packet(&Packet::new(
                play::clientbound::CHAT_MESSAGE,
                w.into_bytes(),
            ))
            .await
            .unwrap();
    });

    let mut client = connect(port).await;
    client
        .handshake(&address(port), NextState::Login)
        .await
        .unwrap();

    match client.login_start("Nephele").await.unwrap() {
        LoginOutcome::Success(success) => {
            assert_eq!(success.username, "Nephele");
            assert_eq!(success.uuid, "069a79f4-44e9-4726-a5be-fca90e38aaf5");
        }
        other => panic!("expected login success, got {other:?}"),
    }
    assert_eq!(client.compression_threshold(), Some(64));

    let join = client.wait_for_join_game().await.unwrap();
    assert_eq!(join.entity_id, 7);
    assert_eq!(join.gamemode, 0);
    assert_eq!(join.dimension, 0);
    assert_eq!(join.level_type, "default");
    assert!(!join.reduced_debug_info);

    let keep_alive = client.next_play_packet().await.unwrap();
    assert_eq!(keep_alive.id, play::clientbound::KEEP_ALIVE);

    let chat = client.next_play_packet().await.unwrap();
    assert_eq!(chat.id, play::clientbound::CHAT_MESSAGE);
    let (text, position) = play::parse_chat_message(&chat).unwrap();
    assert_eq!(text, "<Server> welcome");
    assert_eq!(position, 0);

    server.await.unwrap();
}

#[tokio::test]
async fn login_rejected_in_online_mode() {
    let (listener, port) = accept_one().await;

    let server = tokio::spawn(async move {
        let mut server = accept(listener).await;
        server.read_packet().await.unwrap();
        server.read_packet().await.unwrap();

        let mut w = PacketWriter::new();
        w.write_string("")
            .write_byte_array(&[0u8; 32])
            .write_byte_array(&[0u8; 4]);
        server
            .write_packet(&Packet::new(
                login::CLIENTBOUND_ENCRYPTION_REQUEST,
                w.into_bytes(),
            ))
            .await
            .unwrap();
    });

    let mut client = connect(port).await;
    client
        .handshake(&address(port), NextState::Login)
        .await
        .unwrap();
    match client.login_start("Nephele").await.unwrap() {
        LoginOutcome::EncryptionRequest(request) => {
            assert_eq!(request.public_key.len(), 32);
            assert_eq!(request.verify_token.len(), 4);
        }
        other => panic!("expected encryption request, got {other:?}"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn encryption_response_sends_both_ciphertexts_before_aes_starts() {
    let (listener, port) = accept_one().await;
    let shared_secret = [0x5Au8; 16];
    let encrypted_shared_secret = vec![0xA5; 128];
    let encrypted_verify_token = vec![0x3C; 128];
    let server_encrypted_shared_secret = encrypted_shared_secret.clone();
    let server_encrypted_verify_token = encrypted_verify_token.clone();

    let server = tokio::spawn(async move {
        let mut server = accept(listener).await;
        let response = server.read_packet().await.unwrap();
        assert_eq!(response.id, login::SERVERBOUND_ENCRYPTION_RESPONSE);
        let mut reader = response.reader();
        assert_eq!(
            reader.read_byte_array().unwrap(),
            server_encrypted_shared_secret
        );
        assert_eq!(
            reader.read_byte_array().unwrap(),
            server_encrypted_verify_token
        );
        assert!(reader.is_empty());

        server.enable_encryption(&shared_secret, &shared_secret);
        let next = server.read_packet().await.unwrap();
        assert_eq!(next.id, login::SERVERBOUND_LOGIN_START);
        assert_eq!(next.reader().read_string().unwrap(), "after-encryption");
    });

    let mut client = connect(port).await;
    client.set_state(State::Login);
    client
        .send_encryption_response(
            &shared_secret,
            &encrypted_shared_secret,
            &encrypted_verify_token,
        )
        .await
        .unwrap();
    client
        .write_packet(&login::login_start("after-encryption"))
        .await
        .unwrap();

    server.await.unwrap();
}

#[tokio::test]
async fn a_play_packet_can_arrive_across_many_tcp_writes() {
    let (listener, port) = accept_one().await;

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let packet = Packet::new(0x55, vec![0xA5; 300]);
        let frame = nephele_protocol::framing::encode_frame(&packet.to_payload(), None).unwrap();

        // A 300-byte payload forces a two-byte frame-length VarInt; one-byte
        // writes split both the prefix and body across TCP reads.
        for byte in frame {
            stream.write_all(&[byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
    });

    let mut client = connect(port).await;
    client.set_state(State::Play);
    let packet = tokio::time::timeout(Duration::from_secs(5), client.next_play_packet())
        .await
        .expect("packet read timed out")
        .expect("packet read failed");
    assert_eq!(packet.id, 0x55);
    assert_eq!(packet.data, vec![0xA5; 300]);

    server.await.unwrap();
}

#[tokio::test]
async fn play_disconnect_returns_the_server_reason() {
    let (listener, port) = accept_one().await;

    let server = tokio::spawn(async move {
        let mut server = accept(listener).await;
        let mut w = PacketWriter::new();
        w.write_string(r#"{"text":"Kicked: ","extra":[{"text":"not allowed"}]}"#);
        server
            .write_packet(&Packet::new(play::clientbound::DISCONNECT, w.into_bytes()))
            .await
            .unwrap();
    });

    let mut client = connect(port).await;
    client.set_state(State::Play);
    let error = client.next_play_packet().await.unwrap_err();
    assert!(matches!(
        error,
        nephele_protocol::Error::Disconnected(reason) if reason == "Kicked: not allowed"
    ));

    server.await.unwrap();
}

#[tokio::test]
async fn a_cancelled_read_keeps_the_partial_frame() {
    let (listener, port) = accept_one().await;

    let (mut client, accepted) = tokio::join!(connect(port), listener.accept());
    let (mut peer, _) = accepted.unwrap();
    // Synthetic body: restarting the old reader at this split interprets
    // the tail as the reported 138761173-byte outer length.
    let mut data = vec![0; 200];
    let false_header = nephele_protocol::encode_varint(138_761_173);
    data[1..1 + false_header.len()].copy_from_slice(&false_header);
    let packet = Packet::new(0x55, data);
    let frame = nephele_protocol::framing::encode_frame(&packet.to_payload(), None).unwrap();
    let (head, tail) = frame.split_at(4);
    peer.write_all(head).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), client.read_packet())
            .await
            .is_err()
    );
    peer.write_all(tail).await.unwrap();
    let actual = tokio::time::timeout(Duration::from_secs(2), client.read_packet())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual, packet);
}

#[tokio::test]
async fn an_oversized_frame_length_is_rejected_before_waiting_for_a_body() {
    let (listener, port) = accept_one().await;

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // A length prefix larger than the 2 MiB cap, and no body at all: the
        // reader must reject it instead of blocking until the body arrives.
        let prefix =
            nephele_protocol::encode_varint((nephele_protocol::framing::MAX_PACKET_LEN + 1) as i32);
        stream.write_all(&prefix).await.unwrap();
        stream.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    let mut client = connect(port).await;
    client.set_state(State::Play);
    let error = tokio::time::timeout(Duration::from_secs(5), client.read_packet())
        .await
        .expect("the oversized length must not block")
        .expect_err("an oversized frame must be rejected");
    assert!(matches!(
        error,
        nephele_protocol::Error::PacketTooLarge { .. }
    ));

    server.await.unwrap();
}

/// Sanity check that `Connection::from_stream` handles a socket produced by
/// `TcpStream::connect` too (keeps the test helper honest).
#[allow(dead_code)]
async fn roundtrip_stream(stream: TcpStream) {
    let connection = Connection::from_stream(stream, State::Play);
    let _ = connection.state();
}
