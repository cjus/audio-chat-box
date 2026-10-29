# Local demonstration image. Publishing it distributes a GPL-3.0-or-later program: read
# "Publishing the image" in docs/docker-demo.md first.
# Only the Linux x86-64 native archive below has been checksum-verified.
FROM rust:1.97.1-bookworm AS build
ARG TARGETARCH
RUN test "$TARGETARCH" = amd64
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl bzip2 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /source
COPY . .
RUN mkdir -p /opt/sherpa \
    && curl --fail --show-error --location --retry 3 \
      https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-linux-x64-static-lib.tar.bz2 \
      -o /opt/sherpa/sherpa-onnx-v1.13.8-linux-x64-static-lib.tar.bz2 \
    && echo 'e1fdc5b67530e15741ef897fa5ffff297056f3bf0c6d829a27af9225a4c4b5a6  /opt/sherpa/sherpa-onnx-v1.13.8-linux-x64-static-lib.tar.bz2' | sha256sum --check
ENV SHERPA_ONNX_ARCHIVE_DIR=/opt/sherpa
RUN cargo build --release --locked \
    && cargo build --release --locked --manifest-path demo-web/Cargo.toml
RUN cargo install --locked cargo-about@0.9.2 --features cli \
    && bash tools/notices.sh \
    && printf '%s\n' 'Third-party notices for audio-chat-box-demo (separate HTTP client)' > /tmp/demo-notices.hbs \
    && sed -n '/^Crates$/,$p' tools/notices/notices.hbs >> /tmp/demo-notices.hbs \
    && cargo about generate --manifest-path demo-web/Cargo.toml --locked \
      -c tools/notices/about.toml -o target/licences/DEMO-THIRD-PARTY-NOTICES.txt /tmp/demo-notices.hbs
# Keep this exact project's source and build instructions beside the local binaries.
# This is not a claim that it contains the full corresponding native dependency source.
RUN tar --exclude='./target' --exclude='./demo-web/target' \
    -czf /project-source.tar.gz .

# The models the demo uses, Kokoro and Parakeet, each checked against the size and SHA-256 in
# src/models.rs before it is unpacked (tools/fetch-models.sh). This stage reads nothing else
# from the source, so a change to the code neither downloads them again nor changes their layer.
FROM debian:bookworm-slim AS models
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl bzip2 \
    && rm -rf /var/lib/apt/lists/*
COPY src/models.rs /source/src/models.rs
COPY tools/fetch-models.sh /source/tools/fetch-models.sh
# Keep only the files audio-chat-box reads (src/speak.rs, src/hear.rs) and Kokoro's licence;
# demo-web/MODELS-NOTICE.txt lists them and what is left out.
RUN bash /source/tools/fetch-models.sh /models kokoro parakeet \
    && cd /models/kokoro-multi-lang-v1_0 \
    && find . -mindepth 1 -maxdepth 1 ! -name model.onnx ! -name voices.bin ! -name tokens.txt \
      ! -name lexicon-us-en.txt ! -name espeak-ng-data ! -name LICENSE -exec rm -rf {} + \
    && cd /models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 \
    && find . -mindepth 1 -maxdepth 1 ! -name encoder.int8.onnx ! -name decoder.int8.onnx \
      ! -name joiner.int8.onnx ! -name tokens.txt ! -name bpe.vocab -exec rm -rf {} + \
    && chmod -R a+rX,go-w /models

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 libgomp1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 demo && useradd --uid 10001 --gid demo --no-create-home demo \
    && mkdir -p /usr/share/audio-chat-box
# Read-only and owned by root: audio-chat-box only reads them (it writes Parakeet's bpe.vocab
# only when it is missing or different, and tools/fetch-models.sh wrote it).
COPY --from=models /models /models
COPY demo-web/MODELS-NOTICE.txt /models/NOTICE.txt
COPY demo-web/MODELS-NOTICE.txt /usr/share/audio-chat-box/MODELS-NOTICE.txt
COPY --from=build /source/target/release/audio-chat-box /usr/local/bin/audio-chat-box
COPY --from=build /source/demo-web/target/release/audio-chat-box-demo /usr/local/bin/audio-chat-box-demo
COPY --from=build /source/target/licences/ /usr/share/audio-chat-box/
COPY --from=build /source/licences/ /usr/share/audio-chat-box/native-licences/
COPY --from=build /source/LICENSE-* /source/CREDITS.md /usr/share/audio-chat-box/
COPY --from=build /project-source.tar.gz /usr/share/audio-chat-box/project-source.tar.gz
# The models are in the image: never download one.
ENV AUDIO_CHAT_BOX_BIN=/usr/local/bin/audio-chat-box \
    AUDIO_CHAT_BOX_MODELS=/models \
    AUDIO_CHAT_BOX_OFFLINE=1 \
    DEMO_ORIGIN=http://localhost:8080
# The commit the image was built from, so a pulled image leads back to its exact source. Last,
# so a new commit changes only this metadata, not the layers above.
ARG REVISION=unknown
LABEL org.opencontainers.image.title="audio-chat-box Parrot Service demo" \
      org.opencontainers.image.description="Local browser demo: record, transcribe with Parakeet, read back with Kokoro" \
      org.opencontainers.image.source="https://github.com/cjus/audio-chat-box" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.licenses="GPL-3.0-or-later AND Apache-2.0 AND CC-BY-4.0"
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/audio-chat-box-demo"]
