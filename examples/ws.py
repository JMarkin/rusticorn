import asyncio
import logging
import os

logging.basicConfig()
logging.getLogger().setLevel(os.getenv('LOG_LEVEL', 'DEBUG'))

async def app(scope, receive, send):
    print(scope)

    print(await receive())
    await send({"type": "websocket.accept", "headers": []})

    await send({"type": "websocket.send", "text": "test"})
    print(await receive())
    await send({"type": "websocket.close", "code": 1007 })

if __name__ == '__main__':
    import rusticorn

    rusticorn.run(app, "0.0.0.0:8000")
