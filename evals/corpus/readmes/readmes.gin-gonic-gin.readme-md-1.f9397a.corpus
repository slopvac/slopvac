# Gin Web Framework

<img align="right" width="159px" src="https://raw.githubusercontent.com/gin-gonic/logo/master/color.png">

[![Build Status](https://github.com/gin-gonic/gin/workflows/Run%20Tests/badge.svg?branch=master)](https://github.com/gin-gonic/gin/actions?query=branch%3Amaster)
[![codecov](https://codecov.io/gh/gin-gonic/gin/branch/master/graph/badge.svg)](https://codecov.io/gh/gin-gonic/gin)
[![Go Report Card](https://goreportcard.com/badge/github.com/gin-gonic/gin)](https://goreportcard.com/report/github.com/gin-gonic/gin)
[![GoDoc](https://pkg.go.dev/badge/github.com/gin-gonic/gin?status.svg)](https://pkg.go.dev/github.com/gin-gonic/gin?tab=doc)
[![Join the chat at https://gitter.im/gin-gonic/gin](https://badges.gitter.im/Join%20Chat.svg)](https://gitter.im/gin-gonic/gin?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)
[![Sourcegraph](https://sourcegraph.com/github.com/gin-gonic/gin/-/badge.svg)](https://sourcegraph.com/github.com/gin-gonic/gin?badge)
[![Open Source Helpers](https://www.codetriage.com/gin-gonic/gin/badges/users.svg)](https://www.codetriage.com/gin-gonic/gin)
[![Release](https://img.shields.io/github/release/gin-gonic/gin.svg?style=flat-square)](https://github.com/gin-gonic/gin/releases)
[![TODOs](https://badgen.net/https/api.tickgit.com/badgen/github.com/gin-gonic/gin)](https://www.tickgit.com/browse?repo=github.com/gin-gonic/gin)

Gin is a web framework written in Go (Golang). It features a martini-like API with performance that is up to 40 times faster thanks to [httprouter](https://github.com/julienschmidt/httprouter). If you need performance and good productivity, you will love Gin.

## Contents

- [Gin Web Framework](#gin-web-framework)
  - [Contents](#contents)
  - [Installation](#installation)
  - [Quick start](#quick-start)
  - [Benchmarks](#benchmarks)
  - [Gin v1. stable](#gin-v1-stable)
  - [Build with json replacement](#build-with-json-replacement)
  - [Build without `MsgPack` rendering feature](#build-without-msgpack-rendering-feature)
  - [API Examples](#api-examples)
    - [Using GET, POST, PUT, PATCH, DELETE and OPTIONS](#using-get-post-put-patch-delete-and-options)
    - [Parameters in path](#parameters-in-path)
    - [Querystring parameters](#querystring-parameters)
    - [Multipart/Urlencoded Form](#multiparturlencoded-form)
    - [Another example: query + post form](#another-example-query--post-form)
    - [Map as querystring or postform parameters](#map-as-querystring-or-postform-parameters)
    - [Upload files](#upload-files)
      - [Single file](#single-file)
      - [Multiple files](#multiple-files)
    - [Grouping routes](#grouping-routes)
    - [Blank Gin without middleware by default](#blank-gin-without-middleware-by-default)
    - [Using middleware](#using-middleware)
    - [Custom Recovery behavior](#custom-recovery-behavior)
    - [How to write log file](#how-to-write-log-file)
    - [Custom Log Format](#custom-log-format)
    - [Controlling Log output coloring](#controlling-log-output-coloring)
    - [Model binding and validation](#model-binding-and-validation)
    - [Custom Validators](#custom-validators)
    - [Only Bind Query String](#only-bind-query-string)
    - [Bind Query String or Post Data](#bind-query-string-or-post-data)
    - [Bind Uri](#bind-uri)
    - [Bind Header](#bind-header)
    - [Bind HTML checkboxes](#bind-html-checkboxes)
    - [Multipart/Urlencoded binding](#multiparturlencoded-binding)
    - [XML, JSON, YAML and ProtoBuf rendering](#xml-json-yaml-and-protobuf-rendering)
      - [SecureJSON](#securejson)
      - [JSONP](#jsonp)
      - [AsciiJSON](#asciijson)
      - [PureJSON](#purejson)
    - [Serving static files](#serving-static-files)
    - [Serving data from file](#serving-data-from-file)
    - [Serving data from reader](#serving-data-from-reader)
    - [HTML rendering](#html-rendering)
      - [Custom Template renderer](#custom-template-renderer)
      - [Custom Delimiters](#custom-delimiters)
      - [Custom Template Funcs](#custom-template-funcs)
    - [Multitemplate](#multitemplate)
    - [Redirects](#redirects)
    - [Custom Middleware](#custom-middleware)
    - [Using BasicAuth() middleware](#using-basicauth-middleware)
    - [Goroutines inside a middleware](#goroutines-inside-a-middleware)
    - [Custom HTTP configuration](#custom-http-configuration)
    - [Support Let's Encrypt](#support-lets-encrypt)
    - [Run multiple service using Gin](#run-multiple-service-using-gin)
    - [Graceful shutdown or restart](#graceful-shutdown-or-restart)
      - [Third-party packages](#third-party-packages)
      - [Manually](#manually)
    - [Build a single binary with templates](#build-a-single-binary-with-templates)
    - [Bind form-data request with custom struct](#bind-form-data-request-with-custom-struct)
    - [Try to bind body into different structs](#try-to-bind-body-into-different-structs)
    - [Bind form-data request with custom struct and custom tag](#bind-form-data-request-with-custom-struct-and-custom-tag)
    - [http2 server push](#http2-server-push)
    - [Define format for the log of routes](#define-format-for-the-log-of-routes)
    - [Set and get a cookie](#set-and-get-a-cookie)
  - [Don't trust all proxies](#dont-trust-all-proxies)
  - [Testing](#testing)
  - [Users](#users)

## Installation

To install Gin package, you need to install Go and set your Go workspace first.

1. You first need [Go](https://golang.org/) installed (**version 1.16+ is required**), then you can use the below Go command to install Gin.

```sh
go get -u github.com/gin-gonic/gin
```

2. Import it in your code:

```go
import "github.com/gin-gonic/gin"
```

3. (Optional) Import `net/http`. This is required for example if using constants such as `http.StatusOK`.

```go
import "net/http"
```

## Quick start

```sh
# assume the following codes in example.go file
$ cat example.go
```

```go
package main

import (
  "net/http"

  "github.com/gin-gonic/gin"
)

func main() {
  r := gin.Default()
  r.GET("/ping", func(c *gin.Context) {
    c.JSON(http.StatusOK, gin.H{
      "message": "pong",
    })
  })
  r.Run() // listen and serve on 0.0.0.0:8080 (for windows "localhost:8080")
}
```

```
# run example.go and visit 0.0.0.0:8080/ping (for windows "localhost:8080/ping") on browser
$ go run example.go
```

## Benchmarks

Gin uses a custom version of [HttpRouter](https://github.com/julienschmidt/httprouter)

[See all benchmarks](/BENCHMARKS.md)

| Benchmark name                 |       (1) |             (2) |          (3) |             (4) |
| ------------------------------ | ---------:| ---------------:| ------------:| ---------------:|
| BenchmarkGin_GithubAll         | **43550** | **27364 ns/op** |   **0 B/op** | **0 allocs/op** |
| BenchmarkAce_GithubAll         |     40543 |     29670 ns/op |       0 B/op |     0 allocs/op |
| BenchmarkAero_GithubAll        |     57632 |     20648 ns/op |       0 B/op |     0 allocs/op |
| BenchmarkBear_GithubAll        |      9234 |    216179 ns/op |   86448 B/op |   943 allocs/op |
| BenchmarkBeego_GithubAll       |      7407 |    243496 ns/op |   71456 B/op |   609 allocs/op |
| BenchmarkBone_GithubAll        |       420 |   2922835 ns/op |  720160 B/op |  8620 allocs/op |
| BenchmarkChi_GithubAll         |      7620 |    238331 ns/op |   87696 B/op |   609 allocs/op |
| BenchmarkDenco_GithubAll       |     18355 |     64494 ns/op |   20224 B/op |   167 allocs/op |
| BenchmarkEcho_GithubAll        |     31251 |     38479 ns/op |       0 B/op |     0 allocs/op |
| BenchmarkGocraftWeb_GithubAll  |      4117 |    300062 ns/op |  131656 B/op |  1686 allocs/op |
| BenchmarkGoji_GithubAll        |      3274 |    416158 ns/op |   56112 B/op |   334 allocs/op |
| BenchmarkGojiv2_GithubAll      |      1402 |    870518 ns/op |  352720 B/op |  4321 allocs/op |
| BenchmarkGoJsonRest_GithubAll  |      2976 |    401507 ns/op |  134371 B/op |  2737 allocs/op |
| BenchmarkGoRestful_GithubAll   |       410 |   2913158 ns/op |  910144 B/op |  2938 allocs/op |
| BenchmarkGorillaMux_GithubAll  |       346 |   3384987 ns/op |  251650 B/op |  1994 allocs/op |
| BenchmarkGowwwRouter_GithubAll |     10000 |    143025 ns/op |   72144 B/op |   501 allocs/op |
| BenchmarkHttpRouter_GithubAll  |     55938 |     21360 ns/op |       0 B/op |     0 allocs/op |
| BenchmarkHttpTreeMux_GithubAll |     10000 |    153944 ns/op |   65856 B/op |   671 allocs/op |
| BenchmarkKocha_GithubAll       |     10000 |    106315 ns/op |   23304 B/op |   843 allocs/op |
| BenchmarkLARS_GithubAll        |     47779 |     25084 ns/op |       0 B/op |     0 allocs/op |
| BenchmarkMacaron_GithubAll     |      3266 |    371907 ns/op |  149409 B/op |  1624 allocs/op |
| BenchmarkMartini_GithubAll     |       331 |   3444706 ns/op |  226551 B/op |  2325 allocs/op |
| BenchmarkPat_GithubAll         |       273 |   4381818 ns/op | 1483152 B/op | 26963 allocs/op |
| BenchmarkPossum_GithubAll      |     10000 |    164367 ns/op |   84448 B/op |   609 allocs/op |
| BenchmarkR2router_GithubAll    |     10000 |    160220 ns/op |   77328 B/op |   979 allocs/op |
| BenchmarkRivet_GithubAll       |     14625 |     82453 ns/op |   16272 B/op |   167 allocs/op |
| BenchmarkTango_GithubAll       |      6255 |    279611 ns/op |   63826 B/op |  1618 allocs/op |
| BenchmarkTigerTonic_GithubAll  |      2008 |    687874 ns/op |  193856 B/op |  4474 allocs/op |
| BenchmarkTraffic_GithubAll     |       355 |   3478508 ns/op |  820744 B/op | 14114 allocs/op |
| BenchmarkVulcan_GithubAll      |      6885 |    193333 ns/op |   19894 B/op |   609 allocs/op |

- (1): Total Repetitions achieved in constant time, higher means more confident result
- (2): Single Repetition Duration (ns/op), lower is better
- (3): Heap Memory (B/op), lower is better
- (4): Average Allocations per Repetition (allocs/op), lower is better

## Gin v1. stable

- [x] Zero allocation router.
- [x] Still the fastest http router and framework. From routing to writing.
- [x] Complete suite of unit tests.
- [x] Battle tested.
- [x] API frozen, new releases will not break your code.

## Build with json replacement

Gin uses `encoding/json` as default json package but you can change it by build from other tags.

[jsoniter](https://github.com/json-iterator/go)

```sh
go build -tags=jsoniter .
```

[go-json](https://github.com/goccy/go-json)

```sh
go build -tags=go_json .
```

[sonic](https://github.com/bytedance/sonic) (you have to ensure that your cpu support avx instruction.)

```sh
$ go build -tags="sonic avx" .
```

## Build without `MsgPack` rendering feature

Gin enables `MsgPack` rendering feature by default. But you can disable this feature by specifying `nomsgpack` build tag.

```sh
go build -tags=nomsgpack .
```

This is useful to reduce the binary size of executable files. See the [detail information](https://github.com/gin-gonic/gin/pull/1852).

## API Examples

You can find a number of ready-to-run examples at [Gin examples repository](https://github.com/gin-gonic/examples).

### Using GET, POST, PUT, PATCH, DELETE and OPTIONS

```go
func main() {
  // Creates a gin router with default middleware:
  // logger and recovery (crash-free) middleware
  router := gin.Default()

  router.GET("/someGet", getting)
  router.POST("/somePost", posting)
  router.PUT("/somePut", putting)
  router.DELETE("/someDelete", deleting)
  router.PATCH("/somePatch", patching)
  router.HEAD("/someHead", head)
  router.OPTIONS("/someOptions", options)

  // By default it serves on :8080 unless a
  // PORT environment variable was defined.
  router.Run()
  // router.Run(":3000") for a hard coded port
}
```

### Parameters in path

```go
func main() {
  router := gin.Default()

  // This handler will match /user/john but will not match /user/ or /user
  router.GET("/user/:name", func(c *gin.Context) {
    name := c.Param("name")
    c.String(http.StatusOK, "Hello %s", name)
  })

  // However, this one will match /user/john/ and also /user/john/send
  // If no other routers match /user/john, it will redirect to /user/john/
  router.GET("/user/:name/*action", func(c *gin.Context) {
    name := c.Param("name")
    action := c.Param("action")
    message := name + " is " + action
    c.String(http.StatusOK, message)
  })

  // For each matched request Context will hold the route definition
  router.POST("/user/:name/*action", func(c *gin.Context) {
    b := c.FullPath() == "/user/:name/*action" // true
    c.String(http.StatusOK, "%t", b)
  })

  // This handler will add a new router for /user/groups.
  // Exact routes are resolved before param routes, regardless of the order they were defined.
  // Routes starting with /user/groups are never interpreted as /user/:name/... routes
  router.GET("/user/groups", func(c *gin.Context) {
    c.String(http.StatusOK, "The available groups are [...]")
  })

  router.Run(":8080")
}
```

### Querystring parameters

```go
func main() {
  router := gin.Default()

  // Query string parameters are parsed using the existing underlying request object.
  // The request responds to a url matching:  /welcome?firstname=Jane&lastname=Doe
  router.GET("/welcome", func(c *gin.Context) {
    firstname := c.DefaultQuery("firstname", "Guest")
    lastname := c.Query("lastname") // shortcut for c.Request.URL.Query().Get("lastname")

    c.String(http.StatusOK, "Hello %s %s", firstname, lastname)
  })
  router.Run(":8080")
}
```

### Multipart/Urlencoded Form

```go
func main() {
  router := gin.Default()

  router.POST("/form_post", func(c *gin.Context) {
    message := c.PostForm("message")
    nick := c.DefaultPostForm("nick", "anonymous")

    c.JSON(http.StatusOK, gin.H{
      "status":  "posted",
      "message": message,
      "nick":    nick,
    })
  })
  router.Run(":8080")
}
```

### Another example: query + post form

```sh
POST /post?id=1234&page=1 HTTP/1.1
Content-Type: application/x-www-form-urlencoded

name=manu&message=this_is_great
```

```go
func main() {
  router := gin.Default()

  router.POST("/post", func(c *gin.Context) {

    id := c.Query("id")
    page := c.DefaultQuery("page", "0")
    name := c.PostForm("name")
    message := c.PostForm("message")

    fmt.Printf("id: %s; page: %s; name: %s; message: %s", id, page, name, message)
  })
  router.Run(":8080")
}
```

```sh
id: 1234; page: 1; name: manu; message: this_is_great
```

### Map as querystring or postform parameters

```sh
POST /post?ids[a]=1234&ids[b]=hello HTTP/1.1
Content-Type: application/x-www-form-urlencoded

names[first]=thinkerou&names[second]=tianou
```

```go
func main() {
  router := gin.Default()

  router.POST("/post", func(c *gin.Context) {

    ids := c.QueryMap("ids")
    names := c.PostFormMap("names")

    fmt.Printf("ids: %v; names: %v", ids, names)
  })
  router.Run(":8080")
}
```

```sh
ids: map[b:hello a:1234]; names: map[second:tianou first:thinkerou]
```

### Upload files

#### Single file

References issue [#774](https://github.com/gin-gonic/gin/issues/774) and detail [example code](https://github.com/gin-gonic/examples/tree/master/upload-file/single).

`file.Filename` **SHOULD NOT** be trusted. See [`Content-Disposition` on MDN](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Content-Disposition#Directives) and [#1693](https://github.com/gin-gonic/gin/issues/1693)

> The filename is always optional and must not be used blindly by the application: path information should be stripped, and conversion to the server file system rules should be done.

```go
func main() {
  router := gin.Default()
  // Set a lower memory limit for multipart forms (default is 32 MiB)
  router.MaxMultipartMemory = 8 << 20  // 8 MiB
  router.POST("/upload", func(c *gin.Context) {
    // Single file
    file, _ := c.FormFile("file")
    log.Println(file.Filename)

    // Upload the file to specific dst.
    c.SaveUploadedFile(file, dst)

    c.String(http.StatusOK, fmt.Sprintf("'%s' uploaded!", file.Filename))
  })
  router.Run(":8080")
}
```

How to `curl`:

```bash
curl -X POST http://localhost:8080/upload \
  -F "file=@/Users/appleboy/test.zip" \
  -H "Content-Type: multipart/form-data"
```

#### Multiple files

See the detail [example code](https://github.com/gin-gonic/examples/tree/master/upload-file/multiple).

```go
func main() {
  router := gin.Default()
  // Set a lower memory limit for multipart forms (default is 32 MiB)
  router.MaxMultipartMemory = 8 << 20  // 8 MiB
  router.POST("/upload", func(c *gin.Context) {
    // Multipart form
    form, _ := c.MultipartForm()
    files := form.File["upload[]"]

    for _, file := range files {
      log.Println(file.Filename)

      // Upload the file to specific dst.
      c.SaveUploadedFile(file, dst)
    }
    c.String(http.StatusOK, fmt.Sprintf("%d files uploaded!", len(files)))
  })
  router.Run(":8080")
}
```

How to `curl`:

```bash
curl -X POST http://localhost:8080/upload \
  -F "upload[]=@/Users/appleboy/test1.zip" \
  -F "upload[]=@/Users/appleboy/test2.zip" \
  -H "Content-Type: multipart/form-data"
```

### Grouping routes

```go
func main() {
  router := gin.Default()

  // Simple group: v1
  v1 := router.Group("/v1")
  {
    v1.POST("/login", loginEndpoint)
    v1.POST("/submit", submitEndpoint)
    v1.POST("/read", readEndpoint)
  }

  // Simple group: v2
  v2 := router.Group("/v2")
  {
    v2.POST("/login", loginEndpoint)
    v2.POST("/submit", submitEndpoint)
    v2.POST("/read", readEndpoint)
  }

  router.Run(":8080")
}
```

### Blank Gin without middleware by default

Use

```go
r := gin.New()
```

instead of

```go
// Default With the Logger and Recovery middleware already attached
r := gin.Default()
```

### Using middleware

```go
func main() {
  // Creates a router without any middleware by default
  r := gin.New()

  // Global middleware
  // Logger middleware will write the logs to gin.DefaultWriter even if you set with GIN_MODE=release.
  // By default gin.DefaultWriter = os.Stdout
  r.Use(gin.Logger())

  // Recovery middleware recovers from any panics and writes a 500 if there was one.
  r.Use(gin.Recovery())

  // Per route middleware, you can add as many as you desire.
  r.GET("/benchmark", MyBenchLogger(), benchEndpoint)

  // Authorization group
  // authorized := r.Group("/", AuthRequired())
  // exactly the same as:
  authorized := r.Group("/")
  // per group middleware! in this case we use the custom created
  // AuthRequired() middleware just in the "authorized" group.
  authorized.Use(AuthRequired())
  {
    authorized.POST("/login", loginEndpoint)
    authorized.POST("/submit", submitEndpoint)
    authorized.POST("/read", readEndpoint)

    // nested group
    testing := authorized.Group("testing")
    // visit 0.0.0.0:8080/testing/analytics
    testing.GET("/analytics", analyticsEndpoint)
  }

  // Listen and serve on 0.0.0.0:8080
  r.Run(":8080")
}
```

### Custom Recovery behavior

```go
func main() {
  // Creates a router without any middleware by default
  r := gin.New()

  // Global middleware
  // Logger middleware will write the logs to gin.DefaultWriter even if you set with GIN_MODE=release.
  // By default gin.DefaultWriter = os.Stdout
  r.Use(gin.Logger())

  // Recovery middleware recovers from any panics and writes a 500 if there was one.
  r.Use(gin.CustomRecovery(func(c *gin.Context, recovered interface{}) {
    if err, ok := recovered.(string); ok {
      c.String(http.StatusInternalServerError, fmt.Sprintf("error: %s", err))
    }
    c.AbortWithStatus(http.StatusInternalServerError)
  }))

  r.GET("/panic", func(c *gin.Context) {
    // panic with a string -- the custom middleware could save this to a database or report it to the user
    panic("foo")
  })

  r.GET("/", func(c *gin.Context) {
    c.String(http.StatusOK, "ohai")
  })

  // Listen and serve on 0.0.0.0:8080
  r.Run(":8080")
}
```

### How to write log file

```go
func main() {
    // Disable Console Color, you don't need console color when writing the logs to file.
    gin.DisableConsoleColor()

    // Logging to a file.
    f, _ := os.Create("gin.log")
    gin.DefaultWriter = io.MultiWriter(f)

    // Use the following code if you need to write the logs to file and console at the same time.
    // gin.DefaultWriter = io.MultiWriter(f, os.Stdout)

    router := gin.Default()
    router.GET("/ping", func(c *gin.Context) {
        c.String(http.StatusOK, "pong")
    })

    router.Run(":8080")
}
```

### Custom Log Format

```go
func main() {
  router := gin.New()

  // LoggerWithFormatter middleware will write the logs to gin.DefaultWriter
  // By default gin.DefaultWriter = os.Stdout
  router.Use(gin.LoggerWithFormatter(func(param gin.LogFormatterParams) string {

    // your custom format
    return fmt.Sprintf("%s - [%s] \"%s %s %s %d %s \"%s\" %s\"\n",
        param.ClientIP,
        param.TimeStamp.Format(time.RFC1123),
        param.Method,
        param.Path,
        param.Request.Proto,
        param.StatusCode,
        param.Latency,
        param.Request.UserAgent(),
        param.ErrorMessage,
    )
  }))
  router.Use(gin.Recovery())

  router.GET("/ping", func(c *gin.Context) {
    c.String(http.StatusOK, "pong")
  })

  router.Run(":8080")
}
```

Sample Output

```sh
::1 - [Fri, 07 Dec 2018 17:04:38 JST] "GET /ping HTTP/1.1 200 122.767µs "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_11_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/71.0.3578.80 Safari/537.36" "
```

### Controlling Log output coloring

By default, logs output on console should be colorized depending on the detected TTY.

Never colorize logs:

```go
func main() {
    // Disable log's color
    gin.DisableConsoleColor()

    // Creates a gin router with default middleware:
    // logger and recovery (crash-free) middleware
    router := gin.Default()

    router.GET("/ping", func(c *gin.Context) {
        c.String(http.StatusOK, "pong")
    })

    router.Run(":8080")
}
```

Always colorize logs:

```go
func main() {
    // Force log's color
    gin.ForceConsoleColor()

    // Creates a gin router with default middleware:
    // logger and recovery (crash-free) middleware
    router := gin.Default()

    router.GET("/ping", func(c *gin.Context) {
        c.String(http.StatusOK, "pong")
    })

    router.Run(":8080")
}
```

### Model binding and validation

To bind a request body into a type, use model binding. We currently support binding of JSON, XML, YAML, TOML and standard form values (foo=bar&boo=baz).

Gin uses [**go-playground/validator/v10**](https://github.com/go-playground/validator) for validation. Check the full docs on tags usage [here](https://godoc.org/github.com/go-playground/validator#hdr-Baked_In_Validators_and_Tags).

Note that you need to set the corresponding binding tag on all fields you want to bind. For example, when binding from JSON, set `json:"fieldname"`.

Also, Gin provides two sets of methods for binding:

- **Type** - Must bind
  - **Methods** - `Bind`, `BindJSON`, `BindXML`, `BindQuery`, `BindYAML`, `BindHeader`, `BindTOML`
  - **Behavior** - These methods use `MustBindWith` under the hood. If there is a binding error, the request is aborted with `c.AbortWithError(400, err).SetType(ErrorTypeBind)`. This sets the response status code to 400 and the `Content-Type` header is set to `text/plain; charset=utf-8`. Note that if you try to set the response code after this, it will result in a warning `[GIN-debug] [WARNING] Headers were already written. Wanted to override status code 400 with 422`. If you wish to have greater control over the behavior, consider using the `ShouldBind` equivalent method.
- **Type** - Should bind
  - **Methods** - `ShouldBind`, `ShouldBindJSON`, `ShouldBindXML`, `ShouldBindQuery`, `ShouldBindYAML`, `ShouldBindHeader`, `ShouldBindTOML`,
  - **Behavior** - These methods use `ShouldBindWith` under the hood. If there is a binding error, the error is returned and it is the developer's responsibility to handle the request and error appropriately.

When using the Bind-method, Gin tries to infer the binder depending on the Content-Type header. If you are sure what you are binding, you can use `MustBindWith` or `ShouldBindWith`.

You can also specify that specific fields are required. If a field is decorated with `binding:"required"` and has a empty value when binding, an error will be returned.

```go
// Binding from JSON
type Login struct {
  User     string `form:"user" json:"user" xml:"user"  binding:"required"`
  Password string `form:"password" json:"password" xml:"password" binding:"required"`
}

func main() {
  router := gin.Default()

  // Example for binding JSON ({"user": "manu", "password": "123"})
  router.POST("/loginJSON", func(c *gin.Context) {
    var json Login
    if err := c.ShouldBindJSON(&json); err != nil {
      c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
      return
    }

    if json.User != "manu" || json.Password != "123" {
      c.JSON(http.StatusUnauthorized, gin.H{"status": "unauthorized"})
      return
    }

    c.JSON(http.StatusOK, gin.H{"status": "you are logged in"})
  })

  // Example for binding XML (
  //  <?xml version="1.0" encoding="UTF-8"?>
  //  <root>
  //    <user>manu</user>
  //    <password>123</password>
  //  </root>)
  router.POST("/loginXML", func(c *gin.Context) {
    var xml Login
    if err := c.ShouldBindXML(&xml); err != nil {
      c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
      return
    }

    if xml.User != "manu" || xml.Password != "123" {
      c.JSON(http.StatusUnauthorized, gin.H{"status": "unauthorized"})
      return
    }

    c.JSON(http.StatusOK, gin.H{"status": "you are logged in"})
  })

  // Example for binding a HTML form (user=manu&password=123)
  router.POST("/loginForm", func(c *gin.Context) {
    var form Login
    // This will infer what binder to use depending on the content-type header.
    if err := c.ShouldBind(&form); err != nil {
      c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
      return
    }

    if form.User != "manu" || form.Password != "123" {
      c.JSON(http.StatusUnauthorized, gin.H{"status": "unauthorized"})
      return
    }

    c.JSON(http.StatusOK, gin.H{"status": "you are logged in"})
  })

  // Listen and serve on 0.0.0.0:8080
  router.Run(":8080")
}
```

Sample request

```sh
$ curl -v -X POST \
  http://localhost:8080/loginJSON \
  -H 'content-type: application/json' \
  -d '{ "user": "manu" }'
> POST /loginJSON HTTP/1.1
> Host: localhost:8080
> User-Agent: curl/7.51.0
> Accept: */*
> content-type: application/json
> Content-Length: 18
>
* upload completely sent off: 18 out of 18 bytes
< HTTP/1.1 400 Bad Request
< Content-Type: application/json; charset=utf-8
< Date: Fri, 04 Aug 2017 03:51:31 GMT
< Content-Length: 100
<
{"error":"Key: 'Login.Password' Error:Field validation for 'Password' failed on the 'required' tag"}
```

Skip validate: when running the above example using the above the `curl` command, it returns error. Because the example use `binding:"required"` for `Password`. If use `binding:"-"` for `Password`, then it will not return error when running the above example again.

### Custom Validators

It is also possible to register custom validators. See the [example code](https://github.com/gin-gonic/examples/tree/master/custom-validation/server.go).

```go
package main

import (
  "net/http"
  "time"

  "github.com/gin-gonic/gin"
  "github.com/gin-gonic/gin/binding"
  "github.com/go-playground/validator/v10"
)

// Booking contains binded and validated data.
type Booking struct {
  CheckIn  time.Time `form:"check_in" binding:"required,bookabledate" time_format:"2006-01-02"`
  CheckOut time.Time `form:"check_out" binding:"required,gtfield=CheckIn" time_format:"2006-01-02"`
}

var bookableDate validator.Func = func(fl validator.FieldLevel) bool {
  date, ok := fl.Field().Interface().(time.Time)
  if ok {
    today := time.Now()
    if today.After(date) {
      return false
    }
  }
  return true
}

func main() {
  route := gin.Default()

  if v, ok := binding.Validator.Engine().(*validator.Validate); ok {
    v.RegisterValidation("bookabledate", bookableDate)
  }

  route.GET("/bookable", getBookable)
  route.Run(":8085")
}

func getBookable(c *gin.Context) {
  var b Booking
  if err := c.ShouldBindWith(&b, binding.Query); err == nil {
    c.JSON(http.StatusOK, gin.H{"message": "Booking dates are valid!"})
  } else {
    c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
  }
}
```

```console
$ curl "localhost:8085/bookable?check_in=2030-04-16&check_out=2030-04-17"
{"message":"Booking dates are valid!"}

$ curl "localhost:8085/bookable?check_in=2030-03-10&check_out=2030-03-09"
{"error":"Key: 'Booking.CheckOut' Error:Field validation for 'CheckOut' failed on the 'gtfield' tag"}

$ curl "localhost:8085/bookable?check_in=2000-03-09&check_out=2000-03-10"
{"error":"Key: 'Booking.CheckIn' Error:Field validation for 'CheckIn' failed on the 'bookabledate' tag"}%
```

[Struct level validations](https://github.com/go-playground/validator/releases/tag/v8.7) can also be registered this way.
See the [struct-lvl-validation example](https://github.com/gin-gonic/examples/tree/master/struct-lvl-validations) to learn more.

### Only Bind Query String

`ShouldBindQuery` function only binds the query params and not the post data. See the [detail information](https://github.com/gin-gonic/gin/issues/742#issuecomment-315953017).

```go
package main

import (
  "log"
  "net/http"

  "github.com/gin-gonic/gin"
)

type Person struct {
  Name    string `form:"name"`
  Address string `form:"address"`
}

func main() {
  route := gin.Default()
  route.Any("/testing", startPage)
  route.Run(":8085")
}

func startPage(c *gin.Context) {
  var person Person
  if c.ShouldBindQuery(&person) == nil {
    log.Println("====== Only Bind By Query String ======")
    log.Println(person.Name)
    log.Println(person.Address)
  }
  c.String(http.StatusOK, "Success")
}

```

### Bind Query String or Post Data

