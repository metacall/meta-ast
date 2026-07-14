from metacall import (
    metacall_load_from_file,
    metacall_load_from_memory,
    metacall_load_from_configuration,
    metacall,
)
import callback

# Entry point: loads Go for auth, JS for validation, TS for formatting.
# Also loads a Python sibling (callback) to form an intra-language cycle,
# and loads TS from an inline string (dynamic, no real file).
metacall_load_from_file('go', ['auth.go'])
metacall_load_from_file('node', ['validate.js'])
metacall_load_from_file('ts', ['hash.ts'])
metacall_load_from_file('py', ['callback.py'])
metacall_load_from_memory('ts', 'export const DYNAMIC = 1;')
metacall_load_from_configuration('deploy.conf.json')


def handle_login(username, password):
    # callback.py mirrors this import, closing a same-language cycle.
    token = callback.mint(username)
    is_valid = metacall('validate_input', username, password)
    if not is_valid:
        return {"error": "invalid input"}
    digest = metacall('authenticate', username, password)
    return {
        "token": metacall('format_hash_result', digest, 'sha256'),
        "minted": token,
        "user": username,
    }
