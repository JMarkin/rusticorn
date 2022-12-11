from typing import Union

from fastapi import FastAPI
from pydantic import BaseModel

import asyncio
import logging

FORMAT = '%(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(logging.DEBUG)

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


async def main():
    import rusticorn
    await rusticorn.start_app(app)



if __name__ == '__main__':
    asyncio.run(main())
