use std::{collections::HashMap, sync::atomic::Ordering};

use rocket::{
    fairing::AdHoc, fs::{relative, FileServer}, get, launch, response::Redirect, routes, tokio::sync::RwLock, uri
};
use ws::WebSocket;
use rocket_dyn_templates::{Template, context};

mod chat;

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes![accueil, ws_echo, favicon])
        .mount("/static", FileServer::from(relative!("static")))
        .mount("/chat", routes![chat::subscribe, chat::accueil_chat, chat::get_rooms])
        .attach(Template::fairing())
        .attach(AdHoc::on_shutdown("Chat shutdown", |_rocket| Box::pin(async move {
            chat::GLOBAL_SHUTDOWN_SIGNAL.store(true, Ordering::Release);
            chat::WAKER_CENTRAL.close_connections().await;
        })))
        .manage(chat::Rooms(RwLock::new(HashMap::new())))
}

#[get("/?<name>")]
fn accueil(name: &str) -> Template {
    Template::render(
        "accueil",
        context! {
            name: name
        }
    )
}

#[get("/echo")]
fn ws_echo(ws: WebSocket) -> ws::Stream!['static] {
    ws::Stream! { ws =>
        for await msg in ws {
            yield msg?;
        }
    }
}

#[get("/favicon.ico")]
fn favicon() -> Redirect { Redirect::permanent(uri!("/static/favicon.ico")) }
