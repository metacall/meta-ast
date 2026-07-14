from metacall import (
    metacall_load_from_file,
    metacall_load_from_memory,
    metacall_load_from_package,
    metacall_load_from_configuration,
    metacall,
)
import cache
import queue

# Level 3 exercises the full deploy module end-to-end:
#  - static file loads across py/go/ts/node
#  - metacall_load_from_memory (inline, no file -> External node)
#  - metacall_load_from_package (dependency, no file -> External node)
#  - metacall_load_from_configuration (config-driven expansion)
#  - intra-language cycle (cache <-> queue) collapsing into one py pod
#  - self-loop reference (orchestrator calls itself via metacall)
metacall_load_from_file('go', ['auth.go'])
metacall_load_from_file('node', ['validate.js'])
metacall_load_from_file('ts', ['hash.ts'])
metacall_load_from_file('py', ['cache.py'])
metacall_load_from_memory('ts', 'export const INLINE = 1;')
metacall_load_from_package('node', 'express')
metacall_load_from_configuration('deploy.conf.json')


def handle_login(username, password):
    # Same-language cycle: orchestrator -> cache -> queue -> orchestrator.
    token = cache.mint(username)
    depth = queue.enqueue(token)
    # Self-loop: orchestrator invokes itself through the metacall runtime.
    rebroadcast = metacall('handle_login', username, password)
    is_valid = metacall('validate_input', username, password)
    digest = metacall('authenticate', username, password)
    return {
        "token": metacall('format_hash_result', digest, 'sha256'),
        "depth": depth,
        "rebroadcast": rebroadcast,
        "user": username,
    }
