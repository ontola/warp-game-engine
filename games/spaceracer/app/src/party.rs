use gamenight::{Client, Command, Seat, button};
use serde_json::{Value, json};
use spaceracer::{handling::Input, race::Controls};
pub enum Event {
    Prepare,
    Roster,
    Random,
    Setting(String, Value),
}
pub struct Party {
    pub client: Client,
    pub session: String,
    pub seats: Vec<Seat>,
    pub running: bool,
    pub declarations: Value,
    finished: bool,
}
impl Party {
    pub fn connect(declarations: Value) -> Result<Option<Self>, String> {
        Ok(Client::connect_from_env("spaceracer")?.map(|client| Self {
            client,
            session: String::new(),
            seats: vec![],
            running: false,
            finished: false,
            declarations,
        }))
    }
    pub fn screen(&self) -> app::Screen {
        if !self.client.launched_by_daemon {
            app::Screen::Normal
        } else if self.running {
            app::Screen::Fullscreen
        } else {
            app::Screen::Hidden
        }
    }
    pub fn overlay(&self) {
        let _ = self.client.request_overlay();
    }
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = vec![];
        loop {
            let commands = self.client.poll();
            if commands.is_empty() {
                break;
            }
            let boundary = commands
                .iter()
                .any(|c| matches!(c, Command::Prepare { .. } | Command::Dispose { .. }));
            for cmd in commands {
                match cmd {
                    Command::Connected => {
                        let _ = self.client.declare_settings(self.declarations.clone());
                        let _ = self.client.declare_actions(
                            json!([{"key":"new_race","label":"New random race","confirm":true}]),
                        );
                    }
                    Command::Prepare { session, mut seats } => {
                        seats.sort_by_key(|s| (s.ai, s.index));
                        self.session = session;
                        self.seats = seats;
                        self.running = false;
                        self.finished = false;
                        let _ = self.client.notify_participation(&self.session, false);
                        events.push(Event::Prepare);
                    }
                    Command::Start { session } | Command::Resume { session }
                        if session == self.session =>
                    {
                        self.running = true
                    }
                    Command::Pause { session } if session == self.session => self.running = false,
                    Command::Dispose { session } if session == self.session => {
                        self.running = false;
                        self.session.clear();
                    }
                    Command::Disconnected(_) => self.running = false,
                    Command::Action { key, .. } if key == "new_race" => events.push(Event::Random),
                    Command::Setting { key, value } => events.push(Event::Setting(key, value)),
                    Command::PlayersUpdated { mut seats } | Command::Roster { mut seats } => {
                        seats.sort_by_key(|s| (s.ai, s.index));
                        // Race slots stay stable through disconnect/reorder. Mid-race
                        // joining is declined through participation(false).
                        for old in &mut self.seats {
                            if let Some(new) = seats
                                .iter()
                                .filter(|s| {
                                    s.ai || self.client.state.player(&s.player_id).is_some()
                                })
                                .find(|s| {
                                    if old.player_id.is_empty() {
                                        s.ai && s.index == old.index
                                    } else {
                                        s.player_id == old.player_id
                                    }
                                })
                            {
                                *old = new.clone();
                            } else {
                                old.ai = true;
                                old.controller.clear();
                            }
                        }
                        events.push(Event::Roster);
                    }
                    _ => {}
                }
            }
            if boundary {
                break;
            }
        }
        events
    }
    pub fn ready(&mut self) {
        if !self.session.is_empty() {
            let _ = self.client.notify_ready(&self.session);
        }
    }
    pub fn new_race(&mut self) {
        self.finished = false;
    }
    pub fn finished(&mut self) {
        if !self.finished {
            let _ = self.client.notify_finished(&self.session);
            self.finished = true;
        }
    }
    pub fn controls(&mut self) -> Vec<Controls> {
        let _ = self.client.report_activity();
        let now = std::time::Instant::now();
        self.seats
            .iter()
            .map(|seat| {
                let p = if seat.ai {
                    gamenight::Pad::default()
                } else {
                    self.client.state.pad_for_seat(seat.index, now, false)
                };
                let l = p.stick(0);
                let r = p.stick(1);
                if p.just(button::START) {
                    let _ = self.client.request_overlay();
                }
                Controls {
                    drive: Input {
                        throttle: p.trigger(1),
                        brake: p.trigger(0),
                        steer: l[0],
                        strafe: r[0],
                        trim: (-l[1] - r[1]).clamp(-1., 1.),
                        boost: p.held(button::B),
                    },
                    fire: p.just(button::X),
                    reset: p.just(button::Y),
                    left: p.held(button::LB),
                    right: p.held(button::RB),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        sync::mpsc,
        time::{Duration, Instant},
    };
    use tungstenite::Message;
    #[test]
    fn host_lifecycle_and_opaque_seat_routing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ack_tx, ack_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let hello: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(hello["game"], "spaceracer");
            for value in [
                json!({"type":"welcome","protocol_version":1,"godot_sdk":6,"party":{}}),
                json!({"type":"prepare","game":"spaceracer","session":"test-race","seats":[{"index":3,"controller":"opaque-controller-b","occupant":{"kind":"local","player_id":"test-b"}},{"index":1,"controller":"opaque-controller-a","occupant":{"kind":"local","player_id":"test-a"}}],"players":[{"id":"test-a","name":"Test Pilot A"},{"id":"test-b","name":"Test Pilot B"}]}),
            ] {
                socket
                    .send(Message::Text(value.to_string().into()))
                    .unwrap();
            }
            loop {
                let value: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                if value["type"] == "ready" {
                    assert_eq!(value["session"], "test-race");
                    break;
                }
            }
            for value in [
                json!({"type":"start","session":"test-race"}),
                json!({"type":"controller_frame","controllers":[{"controller":"opaque-controller-b","buttons":0,"axes":[0,0,-16000,0,0,32767]},{"controller":"opaque-controller-a","buttons":0,"axes":[0,0,0,0,0,8192]}]}),
            ] {
                socket
                    .send(Message::Text(value.to_string().into()))
                    .unwrap();
            }
            ack_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            for kind in ["pause", "resume", "dispose"] {
                socket
                    .send(Message::Text(
                        json!({"type":kind,"session":"test-race"})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
                ack_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        });
        let client = Client::connect(gamenight::Config {
            url: format!("ws://{address}"),
            game_id: "spaceracer".into(),
            token: "test-only".into(),
            launched_by_daemon: true,
        })
        .unwrap();
        let mut p = Party {
            client,
            session: String::new(),
            seats: vec![],
            running: false,
            finished: false,
            declarations: json!([]),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stage = 0;
        while Instant::now() < deadline {
            for ev in p.poll() {
                if matches!(ev, Event::Prepare) {
                    assert_eq!(p.seats[0].index, 1);
                    assert_eq!(p.seats[1].index, 3);
                    p.ready();
                }
            }
            match stage {
                0 if p.running => {
                    let c = p.controls();
                    if c.len() == 2 && c[1].drive.throttle > 0.99 {
                        assert!(
                            (c[0].drive.throttle - (8192. / 32767. - 0.06) / 0.94).abs() < 1e-6
                        );
                        assert!(c[1].drive.strafe < -0.39);
                        stage = 1;
                        ack_tx.send(()).unwrap();
                    }
                }
                1 if !p.running => {
                    stage = 2;
                    ack_tx.send(()).unwrap();
                }
                2 if p.running => {
                    stage = 3;
                    ack_tx.send(()).unwrap();
                }
                3 if p.session.is_empty() => {
                    assert!(!p.running);
                    ack_tx.send(()).unwrap();
                    stage = 4;
                    break;
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(stage, 4);
        server.join().unwrap();
    }
}
