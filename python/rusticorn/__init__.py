import asyncio

from .rusticorn import *


def run(app, bind: str = "127.0.0.1:8000", tls: bool = False, cert_path: str = None, private_path: str = None,):

    async def main():
        import rusticorn
        await rusticorn.start_app(app, bind, tls, cert_path, private_path)

    asyncio.run(main())


__doc__ = rusticorn.__doc__
if hasattr(rusticorn, "__all__"):
	__all__ = rusticorn.__all__
