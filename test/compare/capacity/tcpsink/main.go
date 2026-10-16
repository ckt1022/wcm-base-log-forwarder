// tcpsink — raw TCP discard sink，用於 gen 自我壓測（量 gen 本身的輸出上限）。
//
// 行為：接受多條 TCP 連線，讀取並丟棄資料，只統計每秒的行數與 bytes。
// 結束（SIGINT/SIGTERM）時印出總計。每秒統計寫 stdout，可重導向存檔。
//
// 用法：go run main.go [port]（預設 5140）
package main

import (
	"bufio"
	"fmt"
	"net"
	"os"
	"os/signal"
	"sync/atomic"
	"syscall"
	"time"
)

var (
	totalLines int64
	totalBytes int64
)

func handle(c net.Conn) {
	defer c.Close()
	r := bufio.NewReaderSize(c, 1<<20)
	buf := make([]byte, 1<<20)
	var pending int64 // bytes read since last newline count（粗略：以 read chunk 計）
	for {
		n, err := r.Read(buf)
		if n > 0 {
			atomic.AddInt64(&totalBytes, int64(n))
			var lines int64
			for _, b := range buf[:n] {
				if b == '\n' {
					lines++
				}
			}
			atomic.AddInt64(&totalLines, lines)
			pending = 0
		}
		if err != nil {
			_ = pending
			return
		}
	}
}

func main() {
	port := "5140"
	if len(os.Args) > 1 {
		port = os.Args[1]
	}
	ln, err := net.Listen("tcp", "0.0.0.0:"+port)
	if err != nil {
		fmt.Fprintf(os.Stderr, "[tcpsink] listen: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("[tcpsink] listening :%s\n", port)

	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			go handle(c)
		}
	}()

	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)

	start := time.Now()
	tick := time.NewTicker(time.Second)
	var prevLines, prevBytes int64
	for {
		select {
		case <-tick.C:
			l := atomic.LoadInt64(&totalLines)
			b := atomic.LoadInt64(&totalBytes)
			fmt.Printf("[tcpsink] t=%.0fs lines/s=%d MB/s=%.2f total=%d\n",
				time.Since(start).Seconds(), l-prevLines, float64(b-prevBytes)/1e6, l)
			prevLines, prevBytes = l, b
		case <-sig:
			l := atomic.LoadInt64(&totalLines)
			b := atomic.LoadInt64(&totalBytes)
			el := time.Since(start).Seconds()
			fmt.Printf("[tcpsink] done: total_lines=%d total_bytes=%d elapsed=%.1fs avg=%.0f/s avg_mb_s=%.2f\n",
				l, b, el, float64(l)/el, float64(b)/el/1e6)
			return
		}
	}
}
