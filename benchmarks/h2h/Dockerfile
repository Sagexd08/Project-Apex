FROM rust:1-slim
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends \
    python3 python3-venv python3-dev g++ cmake make pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN python3 -m venv /opt/venv && /opt/venv/bin/pip install --no-cache-dir \
    numpy hnswlib "qdrant-client>=1.16,<1.18"
ENV PATH=/opt/venv/bin:/usr/local/cargo/bin:$PATH
WORKDIR /work/benchmarks/h2h
