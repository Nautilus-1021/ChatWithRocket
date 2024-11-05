import { encode } from "./msgpack/index.mjs"

document.addEventListener("DOMContentLoaded", async (event) => {
    const response = await fetch("/chat/rooms");
    const rooms = await response.json();

    const rooms_list = document.getElementById("roomslist");
    rooms.toSorted((a, b) => { a["id"] - b["id"] }).forEach(element => {
        const room_id = element["id"];
        const button_id = `roombutton${room_id}`;

        rooms_list.innerHTML += `<button id="${button_id}" class="roomconnectbtn">${room_id}</button>`;

        document.getElementById(button_id).addEventListener("click", (event) => { connect_ws(room_id) });
    });

    const room_create_form = document.getElementById("roomcreate");
    room_create_form.addEventListener("submit", (event) => {
        event.preventDefault();

        const formdata = new FormData(room_create_form);

        connect_ws(formdata.get("room_id"));
    });
});

function connect_ws(id) {
    const chat_form = document.getElementById("chatform");
    const message_area = document.getElementById("messagearea");
    let ws_socket = new WebSocket(`/chat/subscribe?room_num=${id}`);
    let is_open = false;

    ws_socket.addEventListener("message", (event) => {
        console.log("Received message: ", event.data);
        let message_p = document.createElement("p");
        message_p.innerText = event.data;
        message_area.appendChild(message_p);
    });

    ws_socket.addEventListener("open", (event) => {
        is_open = true;
        let message = `Hello socket ${getRandomInt(0, 10)} !`;
        console.log("Sending message: ", message);
        ws_socket.send(message);
    });

    ws_socket.addEventListener("close", (event) => {
        console.log("Socket closed: ", event.reason);
    });

    chat_form.addEventListener("submit", (event) => {
        event.preventDefault();
        const formdata = new FormData(chat_form);

        ws_socket.send(formdata.get("messagetxt"));
    })
}

function getRandomInt(min, max) {
    return Math.floor(Math.random() * (max - min + 1)) + min;
}

const sleep_async = ms => new Promise(resolve => setTimeout(resolve, ms));