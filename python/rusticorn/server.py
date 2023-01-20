try:
    import uvloop
    uvloop.install()
except ImportError:
    pass
import asyncio

from .rusticorn import *


def run_server(
    app,
    bind="127.0.0.1:8000",
    http_version="http1",
    cert_path=None,
    private_path=None,
    limit_concurrency=None,
    limit_python_execution=None,
    ws_ping_interval=5.0,
    ws_ping_timeout=5.0,
):
    cfg = Config(
        bind,
        http_version,
        cert_path,
        private_path,
        limit_concurrency,
        limit_python_execution,
        ws_ping_interval,
        ws_ping_timeout,
    )

    async def main():
        server = start_server(cfg)
        while not server.stop():
            data = await server.req()
            if data:
                await app(*data)

    asyncio.run(main())
