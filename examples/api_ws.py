import logging
import os

from fastapi import FastAPI, WebSocket
from fastapi.responses import HTMLResponse

logging.basicConfig()
logging.getLogger().setLevel(os.getenv('LOG_LEVEL', 'DEBUG'))

app = FastAPI()

html = """
<!DOCTYPE html>
<html>
    <head>
        <title>Chat</title>
    </head>
    <body>
        <h1>WebSocket Chat</h1>
        <form action="" onsubmit="sendMessage(event)">
            <input type="text" id="messageText" autocomplete="off"/>
            <button>Send</button>
        </form>
        <ul id='messages'>
        </ul>
        <script>
            var ws = new WebSocket("ws://localhost:8000/ws");
            ws.onmessage = function(event) {
                var messages = document.getElementById('messages')
                var message = document.createElement('li')
                var content = document.createTextNode(event.data)
                message.appendChild(content)
                messages.appendChild(message)
            };
            function sendMessage(event) {
                var input = document.getElementById("messageText")
                ws.send(input.value)
                input.value = ''
                event.preventDefault()
            }
        </script>
    </body>
</html>
"""


@app.get("/")
async def get():
    return HTMLResponse(html)


@app.websocket("/ws")
async def websocket_endpoint(websocket: WebSocket):
    print('scope', websocket)
    await websocket.accept()
    while True:
        data = await websocket.receive()
        await websocket.send_text(f"Message recv was: {data}")


if __name__ == '__main__':
    import rusticorn

    rusticorn.run(
        app,
        "0.0.0.0:8000",
        "http1",
    )
