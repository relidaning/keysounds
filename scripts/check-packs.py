"""Read-only check of the default pack required for a fresh startup."""
import json
from pathlib import Path
import sys


def ready(root):
    pack = root / 'nk-cream'
    try:
        config = json.loads((pack / 'config.json').read_text())
        if not config.get('defines'): return False
        sounds = [config.get('sound')] if config.get('key_define_type', 'single') == 'single' else list(config['defines'].values())
        return bool(sounds) and all(isinstance(name, str) and (pack / name).is_file() and (pack / name).stat().st_size for name in sounds)
    except (OSError, ValueError, TypeError):
        return False


if __name__ == '__main__':
    raise SystemExit(0 if ready(Path(sys.argv[1])) else 1)
