import asyncio

from .rusticorn import *


def run(app, bind: str = "127.0.0.1:8000"):

    async def main():
        import rusticorn
        await rusticorn.start_app(app, bind)

    asyncio.run(main())


__doc__ = rusticorn.__doc__
if hasattr(rusticorn, "__all__"):
	__all__ = rusticorn.__all__
