//! Finding a game on the local network without typing a ticket: a host
//! announces itself about once a second, a client lists whom it hears.
//!
//! The beacon goes to the broadcast address and, because Android may filter
//! broadcast and multicast for an app without a Wi-Fi multicast lock (a
//! NativeActivity has no Java to take one), also directly to every address
//! of the host's /24, plus loopback for two games on one machine.
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The UDP port beacons go to.
pub const PORT: u16 = 47_733;
const MAGIC: &str = "WARPLAN1";
const EVERY: Duration = Duration::from_secs(1);
/// A host not heard for this long has gone.
const FORGET: Duration = Duration::from_millis(3500);

/// A game heard on the network; `ticket` joins it.
#[derive(Clone, Debug, PartialEq)]
pub struct LanHost { pub name: String, pub ticket: String }

fn encode(game: &str, host: &LanHost) -> String {
    format!("{MAGIC}\n{game}\n{}\n{}", host.name.replace('\n', " "), host.ticket)
}

/// A beacon for `game`, or None for anything else on the port.
fn decode(game: &str, text: &str) -> Option<LanHost> {
    let mut lines = text.splitn(4, '\n');
    if lines.next()? != MAGIC || lines.next()? != game { return None; }
    let (name, ticket) = (lines.next()?, lines.next()?);
    (!ticket.is_empty()).then(|| LanHost { name: name.to_string(), ticket: ticket.to_string() })
}

/// This machine's address on the network it would route through (connecting
/// a UDP socket sends nothing).
fn local_ipv4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match s.local_addr().ok()? { SocketAddr::V4(a) if !a.ip().is_unspecified() => Some(*a.ip()), _ => None }
}

/// Announces a host until dropped.
pub struct Announcer { stop: Arc<AtomicBool> }

impl Announcer {
    /// `game` tells games apart (its ALPN does nicely).
    pub fn start(game: &str, name: &str, ticket: &str) -> Result<Self, String> { Self::on_port(game, name, ticket, PORT) }

    fn on_port(game: &str, name: &str, ticket: &str, port: u16) -> Result<Self, String> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
        socket.set_broadcast(true).map_err(|e| e.to_string())?;
        let beacon = encode(game, &LanHost { name: name.to_string(), ticket: ticket.to_string() });
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                // Failures are normal here (no network yet, an address that
                // does not exist): the next round tries again.
                let _ = socket.send_to(beacon.as_bytes(), (Ipv4Addr::BROADCAST, port));
                let _ = socket.send_to(beacon.as_bytes(), (Ipv4Addr::LOCALHOST, port));
                if let Some(ip) = local_ipv4() {
                    let [a, b, c, _] = ip.octets();
                    for d in 1..=254 { let _ = socket.send_to(beacon.as_bytes(), (Ipv4Addr::new(a, b, c, d), port)); }
                }
                std::thread::sleep(EVERY);
            }
        });
        Ok(Self { stop })
    }
}

impl Drop for Announcer { fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed); } }

/// Listens for hosts of one game until dropped.
pub struct Listener { hosts: Arc<Mutex<Vec<(LanHost, Instant)>>>, stop: Arc<AtomicBool> }

impl Listener {
    pub fn start(game: &str) -> Result<Self, String> { Self::on_port(game, PORT) }

    fn on_port(game: &str, port: u16) -> Result<Self, String> {
        // Reuse lets a second game on this machine listen too.
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(|e| e.to_string())?;
        socket.set_reuse_address(true).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        socket.set_reuse_port(true).map_err(|e| e.to_string())?;
        socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into()).map_err(|e| e.to_string())?;
        let socket: UdpSocket = socket.into();
        socket.set_read_timeout(Some(Duration::from_millis(250))).map_err(|e| e.to_string())?;
        let hosts: Arc<Mutex<Vec<(LanHost, Instant)>>> = Default::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (heard, stopped, game) = (hosts.clone(), stop.clone(), game.to_string());
        std::thread::spawn(move || {
            let mut buffer = [0u8; 2048];
            while !stopped.load(Ordering::Relaxed) {
                let Ok((n, _)) = socket.recv_from(&mut buffer) else { continue };
                let Some(host) = std::str::from_utf8(&buffer[..n]).ok().and_then(|t| decode(&game, t)) else { continue };
                let mut hosts = heard.lock().unwrap();
                hosts.retain(|h| h.0.ticket != host.ticket);
                hosts.push((host, Instant::now()));
            }
        });
        Ok(Self { hosts, stop })
    }

    /// The hosts heard lately, most recently heard last.
    pub fn hosts(&self) -> Vec<LanHost> {
        let mut hosts = self.hosts.lock().unwrap();
        hosts.retain(|h| h.1.elapsed() < FORGET);
        hosts.iter().map(|h| h.0.clone()).collect()
    }
}

impl Drop for Listener { fn drop(&mut self) { self.stop.store(true, Ordering::Relaxed); } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacons_carry_name_and_ticket_for_their_own_game_only() {
        let host = LanHost { name: "Joep\nQuest".into(), ticket: "abc123".into() };
        let text = encode("warp/growing-guns/4", &host);
        assert_eq!(decode("warp/growing-guns/4", &text), Some(LanHost { name: "Joep Quest".into(), ticket: "abc123".into() }));
        assert_eq!(decode("warp/growing-guns/3", &text), None, "another version is another game");
        assert_eq!(decode("g", "hello"), None);
        assert_eq!(decode("g", &format!("{MAGIC}\ng\nname\n")), None, "no ticket");
    }

    #[test]
    fn two_games_on_one_machine_find_each_other_and_forget_a_host_that_stops() {
        let port = 47_801;
        let listener = Listener::on_port("game", port).unwrap();
        let second = Listener::on_port("game", port).unwrap();
        let other = Listener::on_port("other", port).unwrap();
        let announcer = Announcer::on_port("game", "Quest", "ticket", port).unwrap();
        let until = Instant::now() + Duration::from_secs(4);
        while listener.hosts().is_empty() && Instant::now() < until { std::thread::sleep(Duration::from_millis(20)); }
        assert_eq!(listener.hosts(), vec![LanHost { name: "Quest".into(), ticket: "ticket".into() }]);
        assert!(other.hosts().is_empty());
        drop(second);
        drop(announcer);
        listener.hosts.lock().unwrap()[0].1 -= FORGET;
        assert!(listener.hosts().is_empty());
    }
}
