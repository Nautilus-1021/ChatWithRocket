use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc
    },
    task::{Context, Poll}
};

use rocket::{futures::SinkExt, get, State};
use rocket_dyn_templates::{context, Template};
use ws::{Message, WebSocket};
use rocket::tokio::{
    sync::{broadcast, RwLock},
    task, select, join
};
use rocket::futures::{FutureExt, StreamExt};
use rocket::serde::{Serialize, json::Json};

#[get("/subscribe?<room_num>")]
pub async fn subscribe<'ro>(ws: WebSocket, rooms: &'ro State<Rooms>, room_num: usize) -> ws::Channel<'ro> {
    let res = if let Some(room) = rooms.0.read().await.get(&room_num) {
        Some((room.clone(), room.subscribe()))
    } else { None };

    let (sender, mut receiver) = match res {
        Some(val) => val,
        None => {
            let broadcast_channel = broadcast::channel(5);
            rooms.0.write().await.insert(room_num, broadcast_channel.0.clone());
            broadcast_channel
        }
    };

    ws.channel(move |stream| { Box::pin(async move {
        let (mut ws_sender, mut ws_receiver) = stream.split();
        let should_close_master = Arc::new(AtomicBool::new(false));

        let should_close = should_close_master.clone();
        let inbound = task::spawn(async move {
            let mut should_close_fut = ShouldCloseAsync(should_close.clone()).fuse();

            loop {
                select! {
                    biased;
                    _ = &mut should_close_fut => break,
                    optmsg = ws_receiver.next() => {
                        match optmsg {
                            Some(opterrmsg) => {
                                match opterrmsg {
                                    Ok(msg) => {
                                        match msg {
                                            Message::Text(msgstr) => {
                                                sender.send(msgstr).unwrap();
                                            }
                                            Message::Close(_close_event) => {
                                                should_close.store(true, Ordering::Release);
                                                break
                                            }
                                            _ => ()
                                        }
                                    }
                                    Err(_errmsg) => {
                                        should_close.store(true, Ordering::Release);
                                        break
                                    }
                                }
                            }
                            None => {
                                should_close.store(true, Ordering::Release);
                                break
                            }
                        }
                    }
                }
            }
        });

        let should_close = should_close_master;
        let outbound = task::spawn(async move {
            let mut should_close_fut = ShouldCloseAsync(should_close.clone()).fuse();

            loop {
                select! {
                    biased;
                    _ = &mut should_close_fut => break,
                    opterrmsg = receiver.recv() => {
                        match opterrmsg {
                            Ok(msg) => {
                                ws_sender.send(Message::Text(msg)).await.unwrap();
                            }
                            Err(errmsg) => {
                                match errmsg {
                                    broadcast::error::RecvError::Closed => {
                                        should_close.store(true, Ordering::Release);
                                        break
                                    }
                                    _ => ()
                                }
                            }
                        }
                    }
                }
            }
        });

        let _ = join!(inbound, outbound);

        Ok(())
    })})
}

#[get("/")]
pub fn accueil_chat() -> Template {
    Template::render("chat", context! {})
}

#[get("/rooms")]
pub async fn get_rooms(rooms: &State<Rooms>) -> Json<Vec<RoomDetails>> {
    let rooms = rooms.0
        .read().await.iter()
        .map(|(room_id, room_sender)| { RoomDetails { id: *room_id, num_clients: room_sender.receiver_count() } })
        .collect::<Vec<_>>();
    Json(rooms)
}

#[derive(Serialize)]
#[serde(crate = "rocket::serde")]
pub struct RoomDetails {
    id: usize,
    num_clients: usize
}

pub struct Rooms(pub RwLock<HashMap<usize, broadcast::Sender<String>>>);

pub static GLOBAL_SHUTDOWN_SIGNAL: AtomicBool = AtomicBool::new(false);

struct ShouldCloseAsync(Arc<AtomicBool>);

impl Future for ShouldCloseAsync {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0.load(Ordering::Acquire) || GLOBAL_SHUTDOWN_SIGNAL.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
