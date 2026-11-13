# テスト用ビットストリーム

`tests/test_decode.rs` で使用する、B フレームを含む Annex B 形式のビットストリームです。

## `red-bframe-320x320-h264.h264`

H.264 High Profile、320x320、50 フレームのビットストリームです。B フレームを
含むため、デコード結果の出力順は入力順と一致しません。

次のコマンドで生成した MP4 (ffmpeg 7.1.1) の各サンプルを Annex B 形式へ変換し、
1 サンプルを 1 アクセスユニットとして連結したものです。

```bash
ffmpeg -y -v error -f lavfi -i "color=red:size=320x320:rate=25:duration=2" \
  -c:v libx264 -preset medium -bf 2 -g 50 -b:v 50k -maxrate 50k -bufsize 100k \
  -pix_fmt yuv420p red-bframe-320x320-h264.mp4
```

元の MP4 は sora-rust-sdk のテストデータ (Apache License 2.0) です。

## `cars-320x240.h265`

HEVC Main Profile、320x240、30 フレームのビットストリームです。B フレームを
含むため、デコード結果の出力順は入力順と一致しません。

Intel VPL の `examples/content/cars_320x240.h265` をそのまま使用しています。
Intel VPL は MIT License です。

```
MIT License

Copyright (c) 2020 Intel Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
