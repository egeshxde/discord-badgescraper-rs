use std::fs::{read_to_string, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Duration;
use futures::{StreamExt, SinkExt};
use serde_json::{Value, json};
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, WebSocketStream, MaybeTlsStream};
use tokio::time::sleep;
use tokio::net::TcpStream;
use futures::stream::SplitSink;

#[derive(Deserialize)]
struct Config {
    token: String,
    guild_id: String,
    channel_id: String
}

struct DiscordSocket {
    token: String,
    guild_id: String,
    channel_id: String,
    ranges: Vec<(u64, u64)>,
    last_range: u64,
    packets_recv: u64,
    end_scraping: bool,
    guild_member_count: u64,
}

impl DiscordSocket {
    fn get_ranges(index: u64, multiplier: u64, member_count: u64) -> Vec<(u64, u64)> {
        let initial = index * multiplier;
        let mut v = vec![(initial, initial + 99)];
        if member_count > initial + 99 {
            v.push((initial + 100, initial + 199));
        }
        if !v.iter().any(|r| r.0 == 0 && r.1 == 99) {
            v.insert(0, (0, 99));
        }
        v
    }

    fn write_to_file(line: &str) {
        let mut f = OpenOptions::new().create(true).append(true).open("results.txt").unwrap();
        writeln!(f, "{}", line).ok();
    }

    fn parse_guild_member_list_update(resp: &Value) -> (String, u64, u64, Vec<String>, Vec<Value>, Vec<Value>) {
        let d = &resp["d"];
        let online_count = d["online_count"].as_u64().unwrap_or(0);
        let member_count = d["member_count"].as_u64().unwrap_or(0);
        let guild_id = d["guild_id"].as_str().unwrap_or("").to_string();
        let empty_vec = Vec::new();
        let ops = d["ops"].as_array().unwrap_or(&empty_vec);
        let mut types = Vec::new();
        let mut locs = Vec::new();
        let mut ups = Vec::new();
        for chunk in ops {
            let op = chunk["op"].as_str().unwrap_or("").to_string();
            types.push(op.clone());
            if op == "SYNC" || op == "INVALIDATE" {
                locs.push(chunk["range"].clone());
                if op == "SYNC" {
                    ups.push(chunk["items"].clone());
                } else {
                    ups.push(json!([]));
                }
            } else if op == "INSERT" || op == "UPDATE" || op == "DELETE" {
                locs.push(chunk["index"].clone());
                if op == "DELETE" {
                    ups.push(json!([]));
                } else {
                    ups.push(chunk["item"].clone());
                }
            }
        }
        (guild_id, online_count, member_count, types, locs, ups)
    }

    async fn run(&mut self) {
        let url = "wss://gateway.discord.gg/?encoding=json&v=9";
        let (ws_stream, _) = connect_async(url).await.unwrap();
        let (w_s, mut r) = ws_stream.split();
        let w: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>> = Arc::new(Mutex::new(w_s));
        DiscordSocket::write_to_file("");
        let identify = json!({
            "op": 2,
            "d": {
                "token": self.token,
                "capabilities": 125,
                "properties": {
                    "os": "Windows",
                    "browser": "Firefox",
                    "device": "",
                    "system_locale": "it-IT",
                    "browser_user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:94.0) Gecko/20100101 Firefox/94.0",
                    "browser_version": "94.0",
                    "os_version": "10",
                    "referrer": "",
                    "referring_domain": "",
                    "referrer_current": "",
                    "referring_domain_current": "",
                    "release_channel": "stable",
                    "client_build_number": 103981,
                    "client_event_source": null
                },
                "presence": {
                    "status": "online",
                    "since": 0,
                    "activities": [],
                    "afk": false
                },
                "compress": false,
                "client_state": {
                    "guild_hashes": {},
                    "highest_last_message_id": "0",
                    "read_state_version": 0,
                    "user_guild_settings_version": -1,
                    "user_settings_version": -1
                }
            }
        });
        {
            let mut ws = w.lock().await;
            ws.send(Message::Text(identify.to_string().into())).await.ok();
        }
        let mut heartbeat_task = None;
        let mut ready = false;
        let mut ready_supp = false;
        while let Some(msg) = r.next().await {
            if let Ok(Message::Text(txt)) = msg {
                let txt_str = txt.to_string();
                let decoded: Value = serde_json::from_str(&txt_str).unwrap_or(json!({}));
                let op = decoded["op"].as_u64().unwrap_or(0);
                if op != 11 {
                    self.packets_recv += 1;
                }
                if op == 10 {
                    let heartbeat_interval = decoded["d"]["heartbeat_interval"].as_u64().unwrap_or(45000);
                    let p = self.packets_recv;
                    let ww = Arc::clone(&w);
                    heartbeat_task = Some(tokio::spawn(async move {
                        loop {
                            sleep(Duration::from_millis(heartbeat_interval)).await;
                            let mut ws = ww.lock().await;
                            ws.send(Message::Text(json!({"op": 1, "d": p}).to_string().into())).await.ok();
                        }
                    }));
                }
                if decoded["t"] == "READY" && !ready {
                    ready = true;
                    if let Some(guilds) = decoded["d"]["guilds"].as_array() {
                        for g in guilds {
                            if g["id"].as_str().unwrap_or("") == self.guild_id {
                                self.guild_member_count = g["member_count"].as_u64().unwrap_or(0);
                            }
                        }
                    }
                }
                if decoded["t"] == "READY_SUPPLEMENTAL" && !ready_supp {
                    ready_supp = true;
                    self.ranges = DiscordSocket::get_ranges(0, 100, self.guild_member_count);
                    let mut p = json!({
                        "op": 14,
                        "d": {
                            "guild_id": self.guild_id,
                            "typing": true,
                            "activities": true,
                            "threads": true,
                            "channels": {}
                        }
                    });
                    p["d"]["channels"][&self.channel_id] = serde_json::to_value(self.ranges.iter().map(|r| vec![r.0, r.1]).collect::<Vec<_>>()).unwrap();
                    let mut ws = w.lock().await;
                    ws.send(Message::Text(p.to_string().into())).await.ok();
                }
                if decoded["t"] == "GUILD_MEMBER_LIST_UPDATE" {
                    let (gid, _, _, types, _, updates) = DiscordSocket::parse_guild_member_list_update(&decoded);
                    if gid == self.guild_id && (types.contains(&"SYNC".to_string()) || types.contains(&"UPDATE".to_string())) {
                        for (i, t) in types.iter().enumerate() {
                            let u = &updates[i];
                            if t == "SYNC" && u.as_array().unwrap_or(&Vec::new()).is_empty() {
                                self.end_scraping = true;
                                break;
                            }
                            if t == "SYNC" || t == "UPDATE" || t == "INSERT" {
                                for item in u.as_array().unwrap_or(&Vec::new()) {
                                    let mem = &item["member"];
                                    let user = &mem["user"];
                                    if user.is_object() {
                                        let flags = user["public_flags"].as_u64().unwrap_or(0);
                                        let mut badges = Vec::new();
                                        if (flags & (1 << 1)) != 0 {
                                            badges.push("Partner");
                                        }
                                        if (flags & (1 << 2)) != 0 {
                                            badges.push("HypeSquad Events");
                                        }
                                        if (flags & (1 << 3)) != 0 {
                                            badges.push("Bug Hunter Level 1");
                                        }
                                        if (flags & (1 << 9)) != 0 {
                                            badges.push("Early Supporter");
                                        }
                                        if (flags & (1 << 14)) != 0 {
                                            badges.push("Bug Hunter Level 2");
                                        }
                                        if (flags & (1 << 17)) != 0 {
                                            badges.push("Bot Developer");
                                        }
                                        if !badges.is_empty() {
                                            let username = user["username"].as_str().unwrap_or("");
                                            let id = user["id"].as_str().unwrap_or("");
                                            let line = format!("[SCRAPED] User: \"{}\" | ID: {} | Badges: {}", username, id, badges.join(", "));
                                            println!("{}", line);
                                            DiscordSocket::write_to_file(&line);
                                        }
                                    }
                                }
                            }
                            self.last_range += 1;
                            self.ranges = DiscordSocket::get_ranges(self.last_range, 100, self.guild_member_count);
                            if !self.end_scraping {
                                let p = {
                                    let mut tmp = json!({
                                        "op": 14,
                                        "d": {
                                            "guild_id": self.guild_id,
                                            "typing": true,
                                            "activities": true,
                                            "threads": true,
                                            "channels": {}
                                        }
                                    });
                                    tmp["d"]["channels"][&self.channel_id] = serde_json::to_value(self.ranges.iter().map(|r| vec![r.0, r.1]).collect::<Vec<_>>()).unwrap();
                                    tmp
                                };
                                let ww = Arc::clone(&w);
                                tokio::spawn(async move {
                                    sleep(Duration::from_millis(350)).await;
                                    let mut ws = ww.lock().await;
                                    ws.send(Message::Text(p.to_string().into())).await.ok();
                                });
                            }
                        }
                        if self.end_scraping {
                            {
                                let mut ws = w.lock().await;
                                ws.close().await.ok();
                            }
                            break;
                        }
                    }
                }
            } else if let Ok(Message::Close(_)) = msg {
                break;
            }
        }
        if let Some(t) = heartbeat_task {
            t.abort();
        }
        {
            let mut f = OpenOptions::new().create(true).append(true).open("results.txt").unwrap();
            writeln!(f, "\n\nServer ID: {}\nTotal Members: {}", self.guild_id, self.guild_member_count).ok();
        }
    }
}

#[tokio::main]
async fn main() {
    let cfg: Config = serde_json::from_str(&read_to_string("config.json").unwrap()).unwrap();
    let mut s = DiscordSocket {
        token: cfg.token,
        guild_id: cfg.guild_id,
        channel_id: cfg.channel_id,
        ranges: vec![],
        last_range: 0,
        packets_recv: 0,
        end_scraping: false,
        guild_member_count: 0,
    };
    s.run().await;
    println!("Finished.\nServer ID: {}\nTotal Members: {}", s.guild_id, s.guild_member_count);
}
