import functools


def decorator(func):
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        return func(*args, **kwargs)

    return wrapper


@decorator
def decorated_func(x):
    return x * 2


class Service:
    @staticmethod
    async def process(data):
        pass
