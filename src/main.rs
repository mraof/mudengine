extern crate close_enough;
extern crate ordermap;
extern crate ws;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate serdeconv;
#[macro_use]
extern crate slog;
extern crate sloggers;
extern crate specs;

use close_enough::close_enough;
use ws::{Sender as WsSender, Handler, CloseCode, WebSocket};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};

use specs::prelude::*;

fn main() {
    //Set up logging
    let logger = {
        use std::fs;
        use sloggers::{Config, LoggerConfig};
        let toml = if let Ok(data) = fs::read_to_string("Logging.toml") {
            data
        } else {
            let toml = include_str!("Logging.toml").to_string();
            fs::write("Logging.toml", &toml).expect("Unable to write Logging.toml");
            toml
        };
        let config: LoggerConfig = serdeconv::from_toml_str(&toml).expect("Failed to parse logger config");
        config.build_logger().unwrap()
    };
    
    //Start websocket
    let (sender, receiver) = channel();
    let tick_sender = sender.clone();
    let ws = WebSocket::new(move |out| Server {
        out,
        sender: sender.clone(),
    }).unwrap();
    let handle = ws.broadcaster();
    std::thread::spawn(move || { ws.listen("0.0.0.0:8000").unwrap() });
    
    //Initialize state
    let commands = vec!["say", "tell", "talk", "tail swipe", "kick", "me", "n", "s", "w", "e", "u", "d", "score", "help", "quit"];
    let mut player_names = HashMap::new();
    let mut players: HashMap<u32, Player> = HashMap::new();
    let mut messages: Vec<String> = Vec::new();
    let mut tick_counter = 0;
    info!(logger, "Started");

    std::thread::spawn(move || {
        while let Ok(_) = tick_sender.send(EngineMessage {id: <u32>::max_value(), message: MessageType::Tick}) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
    for message in receiver.iter() {
        use MessageType::*;
        use Message::*;
        match message.message {
            Open(out) => {
                out.send(Chat("Please enter a name".to_string(), 3)).unwrap();
                players.insert(message.id, Player {
                    out,
                    state: PlayerState::Login
                });
            }
            Command(command) => {
                let player = players.get_mut(&message.id).unwrap();
                match player.state {
                    PlayerState::Login => {
                        if !player_names.contains_key(&command) {
                            let character = Character {
                                name: command,
                                health: 44,
                                max_health: 47,
                            };
                            player_names.insert(character.name.clone(), message.id);
                            player.out.send(Health(character.health, character.max_health)).unwrap();
                            for chat_msg in messages.iter() {
                                player.out.send(Chat(chat_msg.clone(), 0)).unwrap();
                            }
                            player.out.broadcast(Chat(format!("{} has joined", character.name), 1)).unwrap();
                            player.state = PlayerState::Playing(character);
                        } else {
                            player.out.send(Chat(format!("The name {} is already taken, do better this time", command), 1)).unwrap();
                        }
                    }
                    PlayerState::Playing(ref mut character) => {
                        let args: Vec<_> = command.split_whitespace().collect();
                        if let Some(&command) = close_enough( &commands, &args[0]) {
                            let chat_msg = format!("{}: {:?}", character.name, command);
                            messages.push(chat_msg.clone());
                            player.out.broadcast(Chat(chat_msg, 2)).unwrap();
                            match command {
                                "kick" => {
                                    player.out.send(Chat(format!("You kick yourself, ouch"), 1)).unwrap();
                                    character.health -= 3;
                                    player.out.send(Health(character.health, character.max_health)).unwrap();
                                }
                                "quit" => {
                                    player.out.send(Chat("No, goodbye".to_string(), 1)).unwrap();
                                    player.out.close(ws::CloseCode::Normal).unwrap();
                                }
                                _ => {
                                    player.out.send(Chat(format!("{} is not implemented yet", command), 1)).unwrap();
                                }
                            }
                        } else {
                            player.out.send(Chat(format!("I don't know what \"{}\" means", args[0]), 1)).unwrap();
                        }
                    }
                }
            }
            Close => {
                let player = players.remove(&message.id).unwrap();
                if let PlayerState::Playing(character) = player.state {
                    player.out.broadcast(Chat(format!("{} has departed", character.name), 1)).unwrap();
                    player_names.remove(&character.name);
                }
                players.remove(&message.id);
            }
            Tick => {
                tick_counter += 1;
            }
        }
    }
}

enum PlayerState {
    Login,
    Playing(Character)
}

struct Player {
    out: WsSender,
    state: PlayerState
}

struct Character {
    name: String,
    health: i32,
    max_health: i32,
}

trait Command {
    fn command();
    fn help() {
    }
}

struct EngineMessage {
    id: u32,
    message: MessageType,
}

enum MessageType {
    ///Connection opened
    Open(WsSender),
    ///Command received from player
    Command(String),
    ///Connection closed
    Close,
    ///Perform a tick of the game world
    Tick,
}

struct Server {
    out: WsSender,
    sender: Sender<EngineMessage>,
}

impl Server {
    fn send(&self, message: MessageType) -> ws::Result<()> {
        self.sender.send(EngineMessage {id: self.out.connection_id(), message}).map_err(|error| ws::Error {kind: ws::ErrorKind::Internal, details: error.to_string().into()})
    }
}

impl Handler for Server {
    fn on_open(&mut self, _: ws::Handshake) -> ws::Result<()> {
        self.send(MessageType::Open(self.out.clone()))
    }

    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        self.send(MessageType::Command(msg.to_string()))
    }

    fn on_close(&mut self, _code: CloseCode, _reason: &str) {
        let _ = self.send(MessageType::Close);
    }
    
    fn on_error(&mut self, err: ws::Error) {
        println!("Error occurred with connection {}: {:?}", self.out.connection_id(), err);
        let _ = self.send(MessageType::Close);
    }
}

#[derive(Debug, Serialize)]
enum Message {
    Chat(String, u32),
    Health(i32, i32),
}

impl From<Message> for ws::Message {
    fn from(msg: Message) -> Self {
        ws::Message::Text(serde_json::to_string(&msg).unwrap())
    }
}
