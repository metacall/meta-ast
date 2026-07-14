from metacall import metacall_load_from_file, metacall

metacall_load_from_file('node', ['add.js'])

def compute(a, b):
    return metacall('add', a, b)
