# 文字の字形データ(glyphs.bin)

AI エージェントが MCP の `draw_shapes` で文字を書くときに使う字形のデータです。

## 出典とライセンス

- 元のデータ: [KanjiVG](https://kanjivg.tagaini.net)(https://github.com/KanjiVG/kanjivg)リリース r20260714 の `kanjivg-20260714-main.zip`(SHA-256: `b5df6cd2bc249dd49b8041eb99e38ba9f8dc6b9ea57962a80084b4a315d1a7fc`)
- KanjiVG: Copyright (C) 2009/2010/2011 Ulrich Apel. [Creative Commons Attribution-Share Alike 3.0](https://creativecommons.org/licenses/by-sa/3.0/)
- `glyphs.bin` は KanjiVG を改変したもので、同じ Creative Commons Attribution-Share Alike 3.0 で配布します

## 改変した内容

`../../tools/convert_kanjivg.py` で変換しました。

- 異体字を除く 6,703 字を使用(漢字・ひらがな・カタカナ・英数字・一部の記号)
- 曲線を折れ線にし、座標を 0.5 単位(109×109 の枠)に丸めた
- KanjiVG に無い次の 30 字を、このプロジェクトで描いて足した(これらも同じライセンスで配布します)

  ```text
  " # $ % & ' ( ) * + - / < = > @ [ \ ] ^ _ ` { | } ~ 「 」 （ ）
  ```

英語の表示(`NOTICE.txt`)は、PC アプリの About で使います。

作り直すとき:

```sh
cd src/bridge
python3 -I tools/convert_kanjivg.py <kanjivg-YYYYMMDD-main.zip> assets/kanjivg/glyphs.bin
```
