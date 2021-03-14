FROM rustlang/rust:nightly as builder

RUN USER=root cargo new --bin gateway
WORKDIR ./gateway
COPY ./Cargo.toml ./Cargo.toml
RUN cargo build --release
RUN rm src/*.rs

ADD . ./

RUN rm ./target/release/deps/gateway*
RUN cargo build --release


FROM debian:buster-slim
ARG APP=/usr/src/app

EXPOSE 3030

ENV TZ=Etc/UTC \
    APP_USER=gateway

RUN groupadd $APP_USER \
    && useradd -g $APP_USER $APP_USER \
    && mkdir -p ${APP}

COPY --from=builder /gateway/target/release/gateway ${APP}/gateway

RUN chown -R $APP_USER:$APP_USER ${APP}

USER $APP_USER
WORKDIR ${APP}

CMD ["./gateway"]