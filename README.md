## Rusticorn

My Implementation [ASGI Server](https://asgi.readthedocs.io). Built on pyo3, hyper, tungstenite.

Thanks [ThibaultLemaire](https://github.com/ThibaultLemaire) and [ChillFish8](https://github.com/ChillFish8).

Motivation:
- I want to learn Rust.
- I want to learn ASGI.
- I want to create server with limit execution for asgi application, uvicorn's backlog and limit-concurrency doesn't works as I mean.

Current status:
- Http and Websocket
- TLS
- Http/2

Planning:
- Lifespan spec
- Extensions: server push, zero copy, http trailers, etc.

See [examples](./examples)
