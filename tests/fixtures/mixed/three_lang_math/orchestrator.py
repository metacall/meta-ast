from metacall import metacall, metacall_load_from_file

# Load JS and Rust math modules
metacall_load_from_file("node", ["math.js"])
metacall_load_from_file("rs", ["math_rs"])


def python_add(a, b):
    return a + b


def compute_triple(a, b, c):
    js_sum = metacall("js_add", a, b)
    rs_mult = metacall("rs_mult", js_sum, c)
    return rs_mult
