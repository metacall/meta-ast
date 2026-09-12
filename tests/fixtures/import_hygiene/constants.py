import json

LIMIT = 10


def encode(value):
    return json.dumps({"limit": LIMIT + value})
