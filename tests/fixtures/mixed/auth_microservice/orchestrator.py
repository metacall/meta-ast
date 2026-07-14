from metacall import metacall_load_from_file, metacall

# Orchestration: Python entry point delegates to Go for auth,
# JS for validation, and TS for result formatting.
metacall_load_from_file('node', ['validate.js'])
metacall_load_from_file('go', ['auth.go'])
metacall_load_from_file('ts', ['hash.ts'])

def handle_login(username, password):
    is_valid = metacall('validate_input', username, password)
    if not is_valid:
        return {"error": "invalid input"}

    token = metacall('authenticate', username, password)
    formatted = metacall('format_hash_result', token, 'sha256')
    return {"token": formatted, "user": username}
