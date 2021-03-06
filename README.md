# source_query_cacher [![Build Status](https://travis-ci.org/In-line/source_query_cacher.svg?branch=master)](https://travis-ci.org/In-line/source_query_cacher) [![Build status](https://ci.appveyor.com/api/projects/status/090sue7e0hyspsk6/branch/master?svg=true)](https://ci.appveyor.com/project/In-line/source-query-cacher/branch/master)

Written in Rust, with love and care using Tokio framework. Should be as fast as hell, or slow as hell. I don't know, but you should try to use it anyway :)

Program is intended to prevent some types of DoS attacks targeted to `SRCDS`/`HLDS` servers by caching some requests before they even arrive to the server. To function properly it needs iptables rule to intercept incoming packets.
```
$ ./target/debug/bin --help
source_query_cacher 0.1.X
Alik Aslanyan <cplusplus256@gmail.com>

USAGE:
    bin [OPTIONS] --list <list>...

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --challenge-number-expire <challenge_number_expire>
            Challenge number expire time in milliseconds. [default: 1000]

    -c, --chunk-size <chunk_size>
            Number of servers to dispatched on the same thread. [default: 5]

        --client-queue-expire <client_queue_expire>
            Client queue expire time in milliseconds. [default: 1000]

        --ignore-unknown-challenge_numbers <ignore_unknown_challenge_numbers>
            Ignore unknown challenge numbers. Some monitorings violate protocol and don't request challenge numbers from
            server. [default: false]
    -l, --list <list>...
            List of strings specified in "PROXY_IP:PORT SERVER_IP:PORT" format

    -p, --update-period <update_period>
            Update period in milliseconds. [default: 1000]


```

Example usage:
```
 $ ./source_query_cacher -p 1000 -c 5 -l "127.0.0.1:27015 127.0.0.1:27016" "127.0.0.1:27018 127.0.0.1:27019"
```

### Credits

Rust community! Especially [Rust Tokio](https://t.me/tokio_rust) and [Rust](https://t.me/rustlang_ru) russian telegram chats for the great help and support ։)
