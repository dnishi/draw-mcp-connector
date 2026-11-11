# draw-mcp-connector
手書きIOのMCPアプリ

## AI エージェントへの登録

PC で Draw MCP Connector を起動したうえで、使う AI エージェントに MCP サーバーとして登録します(初回のみ)。

```sh
# Claude Code
claude mcp add --transport http draw http://127.0.0.1:47101/mcp

# Codex
codex mcp add draw --url http://127.0.0.1:47101/mcp
```

AI エージェントを Docker Desktop のコンテナの中で動かしている場合は、`127.0.0.1` の代わりに `host.docker.internal` を使います(コンテナの中の `127.0.0.1` はコンテナ自身を指すため)。

```sh
claude mcp add --transport http draw http://host.docker.internal:47101/mcp
```

## キャンバスの接続

キャンバスはブラウザで開くページです。iPad・スマートフォン・タブレット・PC など、WebGPU と WebTransport の `serverCertificateHashes` に対応したブラウザ(Safari 26.4 以降、Chrome など)から使えます。

1. メニューバーの Draw MCP Connector のアイコンから「Show QR Code」を選ぶ(AI エージェントに「キャンバスを接続して」と頼んでもよい)
2. 表示された QR コードを、書くのに使う機器のカメラで読み取り、キャンバスを開く
3. PC に出るダイアログで「Allow」を押す(メニューバーのアイコンの丸がオレンジ=許可待ち、緑=接続中)

複数の機器から同時に繋げます。書いた線はすべての機器で共有し、メニューには繋がっている機器ごとに状態と IP アドレスを1行ずつ出します。

許可した後に切れても、1分以内に同じタブで繋ぎ直す(ページの再読み込み、右上のステータス)なら、ダイアログは出ずにそのまま繋がります。タブを閉じた後や1分を過ぎた後は、もう一度「Allow」を押します。別のページがこのページの接続を引き継いで繋ぎ直したときは、このページの接続は切れ、ダイアログで知らせます。

## PC のブラウザで開く

メニューバーの Draw MCP Connector のアイコンから「Open Canvas」を選ぶと、この PC の既定のブラウザでキャンバスが開きます。QR コードから開いたときと同じく、PC に出るダイアログで「Allow」を押すと繋がり、マウスで書けます。

## AI が書く図形と文字

AI エージェントは MCP の `draw_shapes` で、折れ線・長方形・楕円・文字をキャンバスに書けます。点の列ではなく図形や文字列を送るので、AI が使うトークンが少なくて済みます。書いたものは、キャンバスでは手書きと同じ線(筆跡)になります。

## ソースからのビルド

PC アプリ(`src/bridge`)は、ビルドのときの環境変数 `BRIDGE_CANVAS_URL` を、QR コードと「Open Canvas」で開くキャンバスのページとして埋め込みます。自分で配信するキャンバスのページ(HTTPS)を使う場合は、その URL を指定します。

```sh
cd src/bridge
BRIDGE_CANVAS_URL=https://canvas.example.com/ cargo build --release
```

指定しなければ `http://127.0.0.1:5173/` になります。これは同じ PC で動かすキャンバスのページの開発サーバーで、この PC のブラウザ(「Open Canvas」)からだけ開けます。

```sh
cd src/ui
npm ci
npm run dev
```

GitHub Actions(`.github/workflows/bridge.yml`)でビルドするときは、リポジトリの変数 `BRIDGE_CANVAS_URL` の値を使います。

## ライセンス

Draw MCP Connector(このアプリ)は MIT License です(`LICENSE`)。同梱した字形データは下のとおり別のライセンスです。メニューの「About Draw MCP Connector」でも表示します。

アプリに組み込んだクレートのライセンスは、ビルドのときに `src/bridge/tools/third_party_licenses.py` でまとめ、.app の `Contents/Resources/THIRD-PARTY-LICENSES.txt` に入れます。組み込むクレートに許すライセンスは `src/bridge/deny.toml` に書き、CI で cargo-deny が確かめます(脆弱性などの勧告も同時に検査します)。キャンバスのページに組み込んだパッケージのライセンスは、ビルドのときに `src/ui/vite.config.ts` が `licenses.txt` にまとめてページと一緒に配信し、JavaScript の先頭のコメントからその場所を案内します。

文字の字形には [KanjiVG](https://kanjivg.tagaini.net)(Copyright (C) 2009/2010/2011 Ulrich Apel、Creative Commons Attribution-Share Alike 3.0)を変換して使っています。変換した字形データ(`src/bridge/assets/kanjivg/glyphs.bin`)も同じライセンスで配布します。詳しくは `src/bridge/assets/kanjivg/README.md` を見てください。
