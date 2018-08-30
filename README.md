# source_query_cacher
Written in Rust, with love and care using Tokio framework. Should be as fast as hell, or slow as hell. I don't know, but you should try to use it anyway :)

```
$ ./source_query_cacher --help
source_query_cacher 0.1.0
Alik Aslanyan <cplusplus256@gmail.com>

USAGE:
    source_query_cacher [OPTIONS] --list <list>...

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -c, --chunk-size <chunk_size>          Number of servers to dispatched on the same thread. [default: 5]
    -l, --list <list>...                   List of strings specified in "PROXY_IP:PORT SERVER_IP:PORT" format
    -p, --update-period <update_period>    Update period in milliseconds. [default: 1000]

```

Example usage:
```
 $ ./source_query_cacher -p 1000 -c 5 -l "127.0.0.1:27015 127.0.0.1:27016" "127.0.0.1:27018 127.0.0.1:27019"
```
