// UDP transport test — matches the radio::udp module in tacticalmesh-link.
// Usage:
//   link-test tx --iface <iface>   → sends 20 frames and exits
//   link-test rx --iface <iface>   → receives for 10s, prints each frame, exits

use std::net::UdpSocket;
use std::time::{Duration, Instant};

const BASE_PORT: u16 = 42800;
const STREAM_ID: u32 = 0x54455354; // "TEST" — priority 0 stream
const PAYLOAD: &[u8] = b"tacticalmesh-link-test-frame-hello";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 || !["tx", "rx"].contains(&args[1].as_str()) || args[2] != "--iface" {
        eprintln!("Usage: link-test <tx|rx> --iface <iface>");
        std::process::exit(1);
    }
    match args[1].as_str() {
        "tx" => tx(),
        "rx" => rx(),
        _ => unreachable!(),
    }
}

fn tx() {
    let port = BASE_PORT + (STREAM_ID & 0x3) as u16;
    let sock = UdpSocket::bind("0.0.0.0:0").expect("bind tx socket");
    sock.set_broadcast(true).expect("set_broadcast");
    let bcast = format!("255.255.255.255:{port}");
    println!("[tx] sending to {bcast}, stream_id=0x{STREAM_ID:08x}");

    for seq in 0u32..20 {
        let mut pkt = Vec::with_capacity(8 + PAYLOAD.len());
        pkt.extend_from_slice(&STREAM_ID.to_le_bytes());
        pkt.extend_from_slice(&seq.to_le_bytes());
        pkt.extend_from_slice(PAYLOAD);
        match sock.send_to(&pkt, &bcast) {
            Ok(_) => println!("[tx] sent frame seq={seq}"),
            Err(e) => eprintln!("[tx] ERROR seq={seq}: {e}"),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("[tx] done");
}

fn rx() {
    let port = BASE_PORT + (STREAM_ID & 0x3) as u16;
    let sock = UdpSocket::bind(format!("0.0.0.0:{port}")).expect("bind rx socket");
    sock.set_broadcast(true).expect("set_broadcast");
    sock.set_nonblocking(true).expect("set_nonblocking");
    println!("[rx] listening on 0.0.0.0:{port}, stream_id=0x{STREAM_ID:08x}, waiting 10s...");

    let mut buf = vec![0u8; 65507];
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut received = 0usize;

    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, addr)) if n >= 8 => {
                let sid = u32::from_le_bytes(buf[..4].try_into().unwrap());
                if sid != STREAM_ID { continue; }
                let seq = u32::from_le_bytes(buf[4..8].try_into().unwrap());
                received += 1;
                let content = std::str::from_utf8(&buf[8..n]).unwrap_or("<binary>");
                println!("[rx] frame #{received} from={addr} seq={seq} content={content:?}");
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => eprintln!("[rx] ERROR: {e}"),
        }
    }
    println!("[rx] done — received {received} frames in 10s");
}
