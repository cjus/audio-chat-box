# Local demonstration image, not a published distribution. See docs/docker-demo.md.
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

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 libgomp1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 demo && useradd --uid 10001 --gid demo --no-create-home demo \
    && mkdir -p /models /usr/share/audio-chat-box \
    && chown demo:demo /models
COPY --from=build /source/target/release/audio-chat-box /usr/local/bin/audio-chat-box
COPY --from=build /source/demo-web/target/release/audio-chat-box-demo /usr/local/bin/audio-chat-box-demo
COPY --from=build /source/target/licences/ /usr/share/audio-chat-box/
COPY --from=build /source/licences/ /usr/share/audio-chat-box/native-licences/
COPY --from=build /source/LICENSE-* /source/CREDITS.md /usr/share/audio-chat-box/
COPY --from=build /project-source.tar.gz /usr/share/audio-chat-box/project-source.tar.gz
ENV AUDIO_CHAT_BOX_BIN=/usr/local/bin/audio-chat-box \
    AUDIO_CHAT_BOX_MODELS=/models \
    DEMO_ORIGIN=http://localhost:8080
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/audio-chat-box-demo"]
