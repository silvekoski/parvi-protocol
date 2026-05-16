// Diagnostic: raw 802.11 frame reception via PF_PACKET (bypasses pcap).
// Usage: pcap-sniff <iface> [count]

use std::mem::{size_of, zeroed};
use libc::{c_char, c_int};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: pcap-sniff <iface> [count]");
        std::process::exit(1);
    }
    let iface = &args[1];
    let count: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, (libc::ETH_P_ALL as u16).to_be() as c_int) };
    if fd < 0 {
        eprintln!("socket(): {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }

    // Get ifindex via ioctl SIOCGIFINDEX.
    let mut ifr: libc::ifreq = unsafe { zeroed() };
    let bytes = iface.as_bytes();
    let mut name = [0u8; libc::IFNAMSIZ];
    name[..bytes.len()].copy_from_slice(bytes);
    for (i, b) in name.iter().enumerate() {
        ifr.ifr_name[i] = *b as c_char;
    }
    if unsafe { libc::ioctl(fd, libc::SIOCGIFINDEX, &mut ifr) } < 0 {
        eprintln!("SIOCGIFINDEX: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    let ifindex = unsafe { ifr.ifr_ifru.ifru_ifindex };
    println!("iface={iface} ifindex={ifindex}");

    // Bind to specific interface.
    let mut sll: libc::sockaddr_ll = unsafe { zeroed() };
    sll.sll_family = libc::AF_PACKET as u16;
    sll.sll_ifindex = ifindex;
    sll.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
    if unsafe {
        libc::bind(fd, &sll as *const _ as *const libc::sockaddr, size_of::<libc::sockaddr_ll>() as _)
    } < 0 {
        eprintln!("bind: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }

    // Set 5s receive timeout.
    let tv = libc::timeval { tv_sec: 5, tv_usec: 0 };
    unsafe {
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO,
            &tv as *const _ as *const libc::c_void, size_of::<libc::timeval>() as _);
    }

    println!("listening for {} frames (5s timeout)...", count);
    let mut buf = vec![0u8; 2048];
    let mut seen = 0usize;

    while seen < count {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock || e.raw_os_error() == Some(libc::EAGAIN) {
                println!("timeout — no frames in 5s");
                break;
            }
            eprintln!("recv: {e}");
            break;
        }
        let n = n as usize;
        seen += 1;

        let rtap_len = if n >= 4 { u16::from_le_bytes([buf[2], buf[3]]) as usize } else { 0 };
        let (fc0, fc1) = if n > rtap_len + 1 { (buf[rtap_len], buf[rtap_len + 1]) } else { (0, 0) };
        let addr2 = if n >= rtap_len + 16 {
            let s = rtap_len + 10;
            format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                buf[s], buf[s+1], buf[s+2], buf[s+3], buf[s+4], buf[s+5])
        } else { "short".into() };

        let hex: String = buf[..n.min(48)].iter().map(|b| format!("{b:02x}")).collect();
        println!("[{seen}] len={n} rtap={rtap_len} fc={fc0:02x}{fc1:02x} addr2={addr2} {hex}");
    }
    println!("done, seen={seen}");
    unsafe { libc::close(fd); }
}
