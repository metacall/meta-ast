from metacall import metacall

# Mirrors the import in orchestrator.py, forming a same-language cycle
# (py <-> py) that must collapse into one pod via SCC merging.
import orchestrator


def mint(username: str) -> str:
    return f"mint:{username}"
