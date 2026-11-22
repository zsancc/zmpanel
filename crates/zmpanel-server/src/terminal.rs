use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMsg {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Ping,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ServerMsg {
    Output { data: String },
    Error { message: String },
    Exit { message: String },
    Pong,
}

fn pick_default_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        if which_exists("pwsh.exe") {
            return ("pwsh.exe".to_string(), vec!["-NoLogo".to_string()]);
        }
        if which_exists("powershell.exe") {
            return ("powershell.exe".to_string(), vec!["-NoLogo".to_string()]);
        }
        ("cmd.exe".to_string(), vec![])
    }
    #[cfg(not(windows))]
    {
        for candidate in ["/bin/bash", "/usr/bin/bash", "/bin/ash", "/bin/sh"] {
            if Path::new(candidate).exists() {
                return (candidate.to_string(), vec![]);
            }
        }
        ("sh".to_string(), vec![])
    }
}

#[cfg(windows)]
fn which_exists(bin: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if dir.join(bin).is_file() {
                return true;
            }
        }
    }
    false
}

pub async fn handle_terminal_socket(socket: WebSocket, initial_cwd: Option<String>, cols: u16, rows: u16) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: rows.clamp(10, 200),
        cols: cols.clamp(20, 400),
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            let _ = send_json(
                &mut ws_tx,
                &ServerMsg::Error {
                    message: format!("创建 PTY 失败: {}", e),
                },
            )
            .await;
            return;
        }
    };

    let (shell, args) = pick_default_shell();
    let mut cmd = CommandBuilder::new(&shell);
    for a in args {
        cmd.arg(a);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    if let Some(cwd) = initial_cwd
        && !cwd.trim().is_empty()
        && Path::new(&cwd).is_dir()
    {
        cmd.cwd(cwd);
    }

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            let _ = send_json(
                &mut ws_tx,
                &ServerMsg::Error {
                    message: format!("启动终端进程失败 ({}): {}", shell, e),
                },
            )
            .await;
            return;
        }
    };
    drop(pair.slave);

    let mut reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            let _ = child.kill();
            let _ = send_json(
                &mut ws_tx,
                &ServerMsg::Error {
                    message: format!("绑定 PTY 读取流失败: {}", e),
                },
            )
            .await;
            return;
        }
    };

    let mut writer = match pair.master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            let _ = child.kill();
            let _ = send_json(
                &mut ws_tx,
                &ServerMsg::Error {
                    message: format!("绑定 PTY 写入流失败: {}", e),
                },
            )
            .await;
            return;
        }
    };

    let (out_tx, mut out_rx) = mpsc::channel::<ServerMsg>(128);

    // Blocking reader thread -> mpsc channel
    let read_tx = out_tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = read_tx.blocking_send(ServerMsg::Exit {
                        message: "终端会话已结束".to_string(),
                    });
                    break;
                }
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    if read_tx
                        .blocking_send(ServerMsg::Output { data: chunk })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => {
                    let _ = read_tx.blocking_send(ServerMsg::Exit {
                        message: "终端连接已关闭".to_string(),
                    });
                    break;
                }
            }
        }
    });

    let master = pair.master;
    let write_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if send_json(&mut ws_tx, &msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(frame)) = ws_rx.next().await {
        match frame {
            Message::Text(txt) => {
                if let Ok(parsed) = serde_json::from_str::<ClientMsg>(&txt) {
                    match parsed {
                        ClientMsg::Input { data } => {
                            if let Err(e) = writer.write_all(data.as_bytes()) {
                                warn!("PTY write failed: {}", e);
                                break;
                            }
                            let _ = writer.flush();
                        }
                        ClientMsg::Resize { cols, rows } => {
                            let _ = master.resize(PtySize {
                                rows: rows.clamp(5, 200),
                                cols: cols.clamp(10, 400),
                                pixel_width: 0,
                                pixel_height: 0,
                            });
                        }
                        ClientMsg::Ping => {
                            let _ = out_tx.send(ServerMsg::Pong).await;
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    write_task.abort();
}

async fn send_json(
    ws_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMsg,
) -> Result<(), axum::Error> {
    let payload = serde_json::to_string(msg).unwrap_or_default();
    ws_tx.send(Message::Text(payload.into())).await
}
