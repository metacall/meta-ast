def recurse(n):
    if n <= 0:
        return 0
    return recurse(n - 1) + n
