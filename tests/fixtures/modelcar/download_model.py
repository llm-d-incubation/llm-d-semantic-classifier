#!/usr/bin/env python3
import argparse
from huggingface_hub import snapshot_download

p = argparse.ArgumentParser()
p.add_argument('--repo', required=True)
p.add_argument('--revision', required=True)
p.add_argument('--output', required=True)
a = p.parse_args()

snapshot_download(
    repo_id=a.repo,
    revision=a.revision,
    local_dir=a.output,
    allow_patterns=['*.safetensors', '*.json', '*.txt', '1_Pooling/*'],
)
