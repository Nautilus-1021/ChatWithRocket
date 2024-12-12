use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, LazyLock
    },
    task::{Context, Poll, Waker}
};

use rocket::{futures::SinkExt, get, tokio::task::block_in_place, State};
use rocket_dyn_templates::{context, Template};
use ws::{Message, WebSocket};
use rocket::tokio::{
    sync::{broadcast, RwLock},
    task, select, join
};
use rocket::futures::{FutureExt, StreamExt};
use rocket::serde::{Serialize, json::Json};

static CLIENT_COUNTER: AtomicUsize = AtomicUsize::new(0);
pub static WAKER_CENTRAL: LazyLock<WakerCentral> = LazyLock::new(|| {
    WakerCentral(std::sync::Mutex::new(HashMap::new()))
});

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
        let client_id = CLIENT_COUNTER.fetch_add(1, Ordering::Relaxed);

        let should_close = should_close_master.clone();
        let inbound = task::spawn(async move {
            let mut should_close_fut = ShouldCloseAsync {
                client_id,
                client_status: should_close.clone(),
                side: false
            }.fuse();

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
            let mut should_close_fut = ShouldCloseAsync {
                client_id,
                client_status: should_close.clone(),
                side: true
            };

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

struct ShouldCloseAsync{
    client_status: Arc<AtomicBool>,
    client_id: usize,
    side: bool
}

impl Future for ShouldCloseAsync {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        WAKER_CENTRAL.add_waker(self.client_id, cx.waker(), self.side);

        if self.client_status.load(Ordering::Acquire) || GLOBAL_SHUTDOWN_SIGNAL.load(Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

pub struct WakerCentral(std::sync::Mutex<HashMap<usize, (Option<Waker>, Option<Waker>)>>);

impl WakerCentral {
    fn add_waker(&self, client_id: usize, waker_to_register: &Waker, sender_side: bool) {
        let mut wakers = self.0.lock().unwrap();

        if let Some(waker) = wakers.get_mut(&client_id) {
            if sender_side {
                if let Some(waker) = &mut waker.0 {
                    waker.clone_from(waker_to_register);
                } else {
                    waker.0 = Some(waker_to_register.clone());
                }
            } else {
                if let Some(waker) = &mut waker.1 {
                    waker.clone_from(waker_to_register);
                } else {
                    waker.1 = Some(waker_to_register.clone());
                }
            }
        } else {
            let tuple = if sender_side {
                (Some(waker_to_register.clone()), None)
            } else {
                (None, Some(waker_to_register.clone()))
            };

            wakers.insert(client_id, tuple);
        }
    }

    pub async fn close_connections(&self) {
        let mut wakers_hashmap = block_in_place(|| {
            self.0.lock()
        }).unwrap();

        for wakers in wakers_hashmap.values_mut() {
            if let Some(waker) = wakers.0.take() {
                waker.wake();
            }
            if let Some(waker) = wakers.1.take() {
                waker.wake();
            }
        }
    }
}
