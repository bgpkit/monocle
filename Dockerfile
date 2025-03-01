FROM alpine:latest AS build-env

RUN apk add --no-cache cargo pkgconfig libressl-dev
RUN cargo install monocle


FROM alpine:latest
RUN apk add --no-cache gcc libressl-dev
COPY --from=build-env /root/.cargo/bin /root/.cargo/bin

ENTRYPOINT [ "/root/.cargo/bin/monocle" ]
