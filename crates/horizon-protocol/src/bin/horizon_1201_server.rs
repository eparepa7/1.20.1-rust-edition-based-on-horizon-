//! Horizon 1.20.1 1.20.1 Standalone Rust Game Server.
//!
//! Listens on TCP (default 127.0.0.1:25565) and serves Minecraft Java 1.20.1 clients (Protocol 763).

use horizon_protocol::{
    buf::Reader,
    connection::Connection,
    legacy_1_20_1_protocol::{
        ids, Legacy1201GameServer, Legacy1201KeepAlive,
        Legacy1201NextState, Legacy1201ServerSession,
    },
};
use horizon_world::legacy_1_20_1_terrain::Legacy1201World;
use std::{
    net::{TcpListener, TcpStream},
    sync::{
        atomic::Ordering,
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn main() {
    let mut port: u16 = 25565;
    let mut motd = String::from("§aHorizon 1.20.1 1.20.1 §7| §fRust Engine");
    let args: Vec<String> = std::env::args().collect();
    for i in 1..args.len() {
        if args[i] == "--port" && i + 1 < args.len() {
            if let Ok(p) = args[i + 1].parse::<u16>() {
                port = p;
            }
        } else if args[i] == "--motd" && i + 1 < args.len() {
            motd = args[i + 1].clone();
        }
    }

    println!("=======================================================");
    println!("  Horizon 1.20.1 1.20.1 Standalone Rust Game Server (Beta 1.0)");
    println!("  Protocol: 763 | DataVersion: 3465");
    println!("  Listening on: 127.0.0.1:{}", port);
    println!("  MOTD: {}", motd);
    println!("=======================================================");

    // 1. Initialize 1.20.1 world with classic storage and terrain
    let world = Legacy1201World::create_or_open("./world_1201", 12345, "Horizon 1.20.1 1.20.1 World")
        .expect("Failed to initialize world storage");
    let server = Arc::new(Legacy1201GameServer::new(world, port, motd, 20));

    // 2. Start 20 Hz server tick thread (50ms per tick)
    let s_tick = server.clone();
    thread::spawn(move || {
        println!("[Horizon 1.20.1 1.20.1] Server game tick loop active (20 Hz / 50ms).");
        while s_tick.running.load(Ordering::Relaxed) {
            let start = Instant::now();
            s_tick.tick();
            let elapsed = start.elapsed();
            if elapsed < Duration::from_millis(50) {
                thread::sleep(Duration::from_millis(50) - elapsed);
            }
        }
    });

    // 3. Bind TCP socket
    let bind_addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&bind_addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to {}: {}", bind_addr, e);
            std::process::exit(1);
        }
    };

    println!("[Horizon 1.20.1 1.20.1] Ready for Minecraft Java 1.20.1 incoming connections!");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let s = server.clone();
                thread::spawn(move || {
                    handle_client(stream, s);
                });
            }
            Err(e) => {
                eprintln!("[Horizon 1.20.1 1.20.1] Error accepting connection: {}", e);
            }
        }
    }
}

fn handle_client(stream: TcpStream, server: Arc<Legacy1201GameServer>) {
    let peer_addr = stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "unknown".into());
    let conn = match Connection::from_stream(stream) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[Horizon 1.20.1 1.20.1] Connection initialization failed for {}: {}", peer_addr, e);
            return;
        }
    };

    let mut session = Legacy1201ServerSession::new(conn);
    let next_state = match session.handle_handshake() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[Horizon 1.20.1 1.20.1] Handshake failed for {}: {}", peer_addr, e);
            return;
        }
    };

    match next_state {
        Legacy1201NextState::Status => {
            let online = server.players.lock().map(|p| p.len() as i32).unwrap_or(0);
            if let Err(e) = session.handle_status(&server.motd, server.max_players, online) {
                eprintln!("[Horizon 1.20.1 1.20.1] Status ping error for {}: {}", peer_addr, e);
            }
        }
        Legacy1201NextState::Login => {
            // Read login start packet
            let (id, payload) = match session.connection.recv() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[Horizon 1.20.1 1.20.1] Error receiving Login Hello: {}", e);
                    return;
                }
            };
            if id != ids::login::serverbound::HELLO {
                eprintln!("[Horizon 1.20.1 1.20.1] Unexpected packet during login: {:#x}", id);
                return;
            }

            let mut r = Reader::new(&payload);
            let name = match r.string(16) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("[Horizon 1.20.1 1.20.1] Malformed player name: {}", e);
                    return;
                }
            };
            let uuid = if r.bool().unwrap_or(false) { r.uuid().ok() } else { None };

            println!("[Horizon 1.20.1 1.20.1] Client connected: {} from {}", name, peer_addr);

            let mut player = match server.connect_player(&mut session.connection, &name, uuid) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[Horizon 1.20.1 1.20.1] Failed to complete login for {}: {}", name, e);
                    return;
                }
            };

            println!("[Horizon 1.20.1 1.20.1] Player {} entered the world (Spawn: [0, -60, 0])!", name);

            // Play loop: receive actions and send regular KeepAlive
            let mut last_keep_alive = Instant::now();
            let mut keep_alive_seq: i64 = 1;

            while server.running.load(Ordering::Relaxed) {
                // Send keepalive every 15 seconds
                if last_keep_alive.elapsed() >= Duration::from_secs(15) {
                    last_keep_alive = Instant::now();
                    keep_alive_seq += 1;
                    let ka = Legacy1201KeepAlive { id: keep_alive_seq };
                    if let Err(e) = session.connection.send(&ka.encode_clientbound()) {
                        eprintln!("[Horizon 1.20.1 1.20.1] Error sending KeepAlive to {}: {}", name, e);
                        break;
                    }
                }

                match session.connection.recv() {
                    Ok((packet_id, payload)) => {
                        match server.handle_player_packet(&mut player, packet_id, &payload) {
                            Ok(Some(resp_packet)) => {
                                if let Err(e) = session.connection.send(&resp_packet) {
                                    eprintln!("[Horizon 1.20.1 1.20.1] Error sending response to {}: {}", name, e);
                                    break;
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                eprintln!("[Horizon 1.20.1 1.20.1] Packet handling warning for {}: {}", name, e);
                            }
                        }
                    }
                    Err(e) => {
                        if !e.is_timed_out() {
                            println!("[Horizon 1.20.1 1.20.1] Player {} disconnected: {}", name, e);
                            break;
                        }
                    }
                }
            }

            // Unregister player
            if let Ok(mut pm) = server.players.lock() {
                pm.remove(&player.uuid);
            }
        }
    }
}
