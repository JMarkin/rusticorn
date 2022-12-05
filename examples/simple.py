import asyncio
import rusticorn
import logging

FORMAT = '%(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(logging.DEBUG)

async def app(scope, receive, send):
    print(scope, receive, send)
    assert scope['type'] == 'http'
    await send({
        'type': 'http.response.start',
        'status': 200,
        'headers': [
            [b'content-type', b'text/plain'],
        ],
    })
    await send({
        'type': 'http.response.body',
        'body': b'Hello, world!',
    })

async def main():
    await rusticorn.start_app(app)



if __name__ == '__main__':
    asyncio.run(main())
