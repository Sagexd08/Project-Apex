FROM ann-benchmarks

RUN apt-get update && apt-get install -y --no-install-recommends curl build-essential git ca-certificates && apt-get clean
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"
RUN pip install maturin numpy

ARG ATLAS_REF=main
RUN git clone https://github.com/Sagexd08/Project-Apex.git /atlas && cd /atlas && git checkout ${ATLAS_REF}
RUN cd /atlas/adapters/atlas-py && RUSTFLAGS="-C target-cpu=native" maturin build --release -o /wheels && pip install /wheels/*.whl

RUN python -c 'import atlas_ann'
