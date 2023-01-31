import importlib

import click

from .server import run_server


@click.command(context_settings={"auto_envvar_prefix": "RUSTICORN"})
@click.argument('application')
@click.option('--bind', default="127.0.0.1:8000", help='bind')
@click.option('--http-version',
              type=click.Choice(['http1', 'http2'], case_sensitive=True),
              default="http1")
@click.option('--cert-path', required=False)
@click.option('--private-path', required=False)
@click.option('--limit-concurrency', default=None)
@click.option('--limit-python-execution', default=None)
@click.option('--ws-ping-interval', default=20.0)
@click.option('--ws-ping-timeout', default=20.0)
def run(application,
        bind="127.0.0.1:8000",
        http_version="http1",
        cert_path=None,
        private_path=None,
        limit_concurrency=None,
        limit_python_execution=None,
        ws_ping_interval=5.0,
        ws_ping_timeout=5.0
        ):
    module, app = application.split(':')
    app = getattr(importlib.import_module(module), app, None)
    if app is None:
        raise ImportError(application)
    run_server(
        app,
        bind,
        http_version,
        cert_path,
        private_path,
        limit_concurrency,
        limit_python_execution,
        ws_ping_interval,
        ws_ping_timeout,
    )


if __name__ == '__main__':
    run()
