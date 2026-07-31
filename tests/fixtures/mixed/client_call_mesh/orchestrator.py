from metacall import metacall, metacall_load_from_file

# Load the Node module, then invoke its exports through the client API.
metacall_load_from_file("node", ["math.js"])


def compute_total(units, price):
    return metacall("multiply", units, price)


def missing_feature(value):
    return metacall("no_such_function", value)
