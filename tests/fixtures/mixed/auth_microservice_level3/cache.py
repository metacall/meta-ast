import queue

# Same-language cycle leg: cache depends on queue, queue depends on cache.
from queue import enqueue


def mint(username):
    token = _sign(username)
    enqueue(token)
    return token


def establish(item):
    return _sign(item)


def _sign(value):
    return f"sig:{value}"
