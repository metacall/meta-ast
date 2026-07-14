import cache

# Same-language cycle leg: queue depends on cache, cache depends on queue.
from cache import establish


def enqueue(item):
    establish(item)
    return queue_len(item)


def queue_len(item):
    return len(item)
