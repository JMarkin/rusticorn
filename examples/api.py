import asyncio
import logging
import os
from typing import Union

from fastapi import FastAPI
from pydantic import BaseModel

FORMAT = '%(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(os.getenv('LOG_LEVEL', 'DEBUG'))

app = FastAPI()


class Item(BaseModel):
    name: str
    price: float
    is_offer: Union[bool, None] = None


@app.get("/")
def read_root():
    return {"Hello": "World"}


@app.get("/items/{item_id}")
def read_item(item_id: int, q: Union[str, None] = None):
    return {"item_id": item_id, "q": q}


@app.put("/items/{item_id}")
def update_item(item_id: int, item: Item):
    return {"item_name": item.name, "item_id": item_id}


if __name__ == '__main__':
    import rusticorn

    rusticorn.run(app, "0.0.0.0:8000", True, "./examples/certs/localhost.pem", "./examples/certs/localhost-key.pem")
