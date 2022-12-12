from locust import FastHttpUser, task
from random import randbytes

data = randbytes(1000)

class ReceiveUser(FastHttpUser):
    @task
    def test(self):
        self.client.post("/", data=data)
