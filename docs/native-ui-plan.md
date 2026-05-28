# Sujay Native UI 計画書

## 概要

Electron ベースの DJ アプリ「Sujay」の DJコンソール部分（波形、レベルメーター、ボタン、フェーダー等）を、ネイティブ GPU 描画に置き換えて高速化する。ライブラリ画面や設定画面などの通常 UI は React (HTML/CSS) のまま維持する。

## 現在の状態

### 完了済み（POC）

- `packages/ui/` に NAPI-RS ベースのネイティブアドオン `@sujay/ui` を作成済み
- Electron の `BrowserWindow.getNativeWindowHandle()` から取得した NSView に、Rust 側から NSView を `addSubview` して埋め込むことに成功
- 赤い四角形が Electron ウィンドウ内に表示されることを確認済み（スクリーンショットあり）

### 直近の実装反映（2026-04）

- React はライブラリ領域を残し、コンソール領域は native overlay で描画する構成に移行中
- macOS は wgpu + CALayer ベースの描画が動作し、黒画面は解消済み
- 波形データ転送は `waveformComplete` 時の全結合を避け、`waveformChunk` 受信時に段階ダウンサンプルする方式へ変更済み（Deck B load 時の固まり対策）
- ズーム波形のレイアウトは「上段左右デッキ領域」に合わせる必要があることを確認（単純な上下 50/50 分割は UI を覆って破綻する）
- `egui` 依存は入っているが、現時点の稼働描画は custom wgpu renderer であり、egui UI 本体は未実装

### 現在のファイル構成

```
packages/ui/
├── package.json          # @sujay/ui, NAPI-RS 設定
├── Cargo.toml            # cocoa/objc + wgpu + (egui依存はあるが未使用)
├── build.rs              # napi_build::setup()
├── rustfmt.toml          # tab_spaces = 2
├── .cargo/config.toml    # Windows CRT 設定
└── src/
  ├── lib.rs                      # NAPI エクスポート
  ├── renderer.rs                 # macOS native console renderer（現行）
  ├── renderer_macos.rs           # macOS wgpu renderer（共有化済み）
  ├── renderer_windows.rs         # Windows wgpu renderer
  ├── renderer_wgpu_shared.rs     # macOS/Windows 共通 wgpu ヘルパー
  ├── waveform.wgsl               # 波形シェーダー
  └── ui_state.rs                 # console visual state 共通型
```

### app/src/main.ts のテストコード

`createWindow()` 末尾に `did-finish-load` イベントで `@sujay/ui` の `attach()` を呼ぶテストコードを追加済み。本実装時に置き換えること。

```typescript
mainWindow.webContents.once('did-finish-load', () => {
  const nativeUI = require('@sujay/ui');
  const handle = mainWindow.getNativeWindowHandle();
  nativeUI.attach(handle, 100, 100, 200, 100);
});
```

## アーキテクチャ

```
Electron BrowserWindow
├── React (HTML/CSS) ← ライブラリ、設定画面など（変更なし）
└── NSView (contentView)
     └── Native GPU View (@sujay/ui が addSubview)
          ├── Metal/wgpu で描画
          │   ├── 波形表示 (WaveformFull, WaveformZoom 相当)
          │   ├── レベルメーター
          │   └── ビジュアルフィードバック
          └── UI コントロール
              ├── Play/Cue ボタン
              ├── EQ ノブ
              ├── フェーダー
              └── ループコントロール
```

## 実装計画

### 直近で得た運用知見（再発防止）

- **renderer 切替時は API 互換を必ず維持する**: `lib.rs` が呼ぶ `attach` や `set_console_state` の型契約がずれると `cargo check` が落ち、React 側を隠している構成では即黒画面になる
- **見た目レイアウトと描画レイヤー座標を一致させる**: 波形コンテナを画面全体の機械分割で置くと、ボタンやコンソール装飾が覆われて「表示されているのに見えない」状態を作る
- **重い処理は chunk 到着時に分散する**: 完了イベントで一括処理すると main process が詰まりやすい。ダウンサンプルなどは受信段階で小分けに行う
- **Native addon の更新反映には再ビルド＋再起動が必要**: `npx napi build --platform` 後にアプリを再起動しないと古いバイナリを読み続ける場合がある

### Phase 1: 座標系とサイズ同期

**目標**: ネイティブビューを HTML の div と正確に重ねる

- [x] macOS 座標系の修正（左下原点 → 左上原点への変換）
  - 親ビューの高さを取得し、`y = parentHeight - y - height` で変換
- [x] レンダラー側で `ResizeObserver` を使い、コンソール領域の div の位置・サイズを監視
- [x] IPC (`webContents.send` / `ipcMain.handle`) でレンダラー → メイン → `@sujay/ui` にフレーム情報を伝達
- [x] ウィンドウリサイズ時の追従

### Phase 2: Metal/wgpu 描画

**目標**: 赤い四角の代わりに wgpu で GPU 描画する

- [x] `Cargo.toml` に `wgpu` 依存を追加
- [x] `macos.rs` の NSView に `CAMetalLayer` をセットアップ
- [x] wgpu の `Surface` を `raw-window-handle` 経由で NSView から作成
- [x] シンプルなクリアカラー描画で動作確認
- [x] `requestAnimationFrame` 相当のレンダーループを別スレッドで実装（`CVDisplayLink` または手動ループ）

### Phase 3: 波形レンダリング移植

**目標**: 既存の WebGL 波形描画を wgpu シェーダーに移植

#### peakform での実装知見（WebGL プロトタイプ）

peakform パッケージで WebGL2 シェーダーベースの波形描画を実装・検証した結果、以下の知見を得た：

- **2パスレンダリングが必須**: 全ピクセルで生サンプルの max-abs ループを回すと GPU 負荷が高すぎる（640K pixels × 256 fetches/frame）。Pass 1 で numBars 個のフラグメントだけで peak 計算→FBO、Pass 2 で peak テクスチャから 1 フェッチ/bar で描画、が正解。wgpu ではコンピュートシェーダーで Pass 1 を置き換えるとさらに効率的。
- **タイムライン固定バー + サブピクセルオフセット**: バーをサンプル空間に固定し、スクロール時はバー全体をサブピクセルずらし（`uFirstBarPx` uniform）。ピクセルグリッド固定だとデータがバー下をスクロール→ガタつく。
- **オーバーレイはシェーダー内で統合可能**: マーカー（テクスチャ、最大2048本）、リージョン（uniform配列、最大8個）、進捗線、センターラインすべてフラグメントシェーダー内で処理。Canvas2D オーバーレイ不要。SDF roundedBox + smoothstep で AA つきバー角丸も実現。
- **再生位置の更新レート**: Rust エンジンが 30fps で state emit → IPC → React setState の経路ではジッターが大きい。ネイティブ UI では CVDisplayLink コールバックで直接 position を読めるため、この問題は解消される見込み。
- **Deck A/B 同期**: 複数波形の描画タイミングがずれるとデッキ間で視覚的ドリフトが生じる。同一フレームで全デッキを描画する必要あり（peakform では共有 rAF ティッカーで対処、wgpu では単一レンダーループで自然に解決）。

#### タスク

- [ ] 既存の peakform GLSL を参考に WGSL シェーダーを作成
  - Pass 1: ピーク計算（コンピュートシェーダーで実装、FBO ではなくストレージバッファ）
  - Pass 2: 波形描画（peak バッファから 1 フェッチ/bar + マーカー/リージョン/進捗線）
- [ ] PCM データは `@sujay/audio` 内で既に保持済み。同一プロセス内の Rust channel で `@sujay/ui` に渡す（Phase 5 で検討の option 3 を初期から採用する方が良い可能性）
- [ ] スクロール（タイムライン固定バー + サブピクセルオフセット方式）
- [ ] マーカー描画（ビート位置をバッファで渡し、シェーダー内でループ）
- [ ] ループリージョン描画（uniform で start/end/color を渡す）
- [ ] ズーム波形とコンソール装飾のレイアウト定数を共通化し、片方修正で破綻しない構成にする

### Phase 4: UI コントロール

**目標**: ボタン・ノブ・フェーダーをネイティブ描画

ここで gpui の採用を再検討する。選択肢:

#### Option A: gpui (crates.io 0.2.2)
- gpui の `App` をバックグラウンドで動かす方法を模索
- 問題: gpui は自身のイベントループを要求する。Electron のイベントループと共存できるか要調査
- gpui-component (Longbridge) の 60+ コンポーネントを活用可能

#### Option B: 自前描画
- wgpu で直接ボタン・スライダー・ノブを描画
- ヒットテスト、マウスイベント処理を自前実装
- 工数は大きいが、依存が少なく制御しやすい

#### Option C: iced を組み込み
- iced は wgpu ベースで、ウィジェットが充実
- `iced_wgpu` レンダラーを使い、既存の wgpu Surface 上に描画
- gpui より組み込みやすい可能性あり

#### 共通タスク
- [ ] マウスイベントの NSView → UI フレームワークへのルーティング
- [ ] Play/Cue ボタン
- [ ] ボリュームスライダー / クロスフェーダー
- [ ] EQ ノブ (Hi/Mid/Lo)
- [ ] ループコントロール (In/Out/Size)
- [ ] BPM 表示
- [ ] イベントを IPC 経由で Electron 側に通知
- [ ] 「見えるが操作できない」事故を避けるため、装飾レイヤーと操作レイヤーの z-order / hit-test 方針を明文化する

### Phase 5: オーディオエンジン連携

**目標**: `@sujay/audio` と `@sujay/ui` の間でデータを効率的にやり取り

#### 現状の構成
- `@sujay/audio` は Worker スレッドで動作
- 状態更新は Worker → Main → Renderer (IPC) で伝達
- 波形データは `waveform-chunk` イベントでストリーミング

#### 連携方法の選択肢

1. **JS 経由の中継（最も安全）**
   - audio worker → main process → `@sujay/ui` の NAPI 関数呼び出し
   - 既存のアーキテクチャを壊さない
   - レイテンシはやや増えるが実用上問題ない可能性

2. **SharedArrayBuffer（高速）**
   - audio が書き込み、ui が毎フレーム読むリングバッファ
   - ロックフリーで高速
   - 波形データ、レベルメーターデータに適する

3. **Rust 内 channel（最速、将来的に）**
   - 2 つのアドオンを 1 つに統合するか、共有ライブラリ経由で通信
   - IPC オーバーヘッドゼロ
   - 大幅なリファクタリングが必要

**推奨**: Phase 5 初期は **1 (JS 中継)** で動かし、パフォーマンス問題が出たら **2 (SharedArrayBuffer)** に移行。

### Phase 6: React 側の整理

- [ ] `WaveformFull.tsx`, `WaveformZoom.tsx` のネイティブ版への切り替え
- [ ] `LevelMeter.tsx` のネイティブ版への切り替え
- [ ] `Console.tsx` の DJコントロール部分をネイティブ版に委譲
- [ ] peakform パッケージの段階的廃止（ネイティブ側に機能移行後）
- [ ] ネイティブビュー領域の div を透明プレースホルダーとして維持（サイズ同期用）

## 技術的注意事項

### macOS 座標系
- NSView の座標は左下原点（Y 軸が上向き）
- HTML/CSS は左上原点
- `y = parentView.bounds.height - htmlY - viewHeight` で変換

### Electron の getNativeWindowHandle()
- macOS では NSView ポインタを Buffer として返す
- aarch64 では 8 バイト、x86 では 4 バイト
- `u64::from_ne_bytes()` でポインタに変換

### gpui 0.2.2 (crates.io)
- Zed 公式リリース。stable Rust (1.88+) でビルド可能
- Glass-HQ フォーク (git main) は 2026-04 時点でビルドが壊れている
- gpui は独自のイベントループを持つため、Electron 内での共存に工夫が必要
- `WindowKind::Child` や `PlatformSurface` は crates.io 版には**存在しない**

### 既存パッケージとの関係
- `@sujay/audio` (packages/audio): Rust NAPI-RS アドオン。CPAL ベースのオーディオエンジン
- `peakform` (packages/peakform): TypeScript。WebGL 波形レンダラー。Phase 3 で機能をネイティブ側に移植
- `@sujay/ui` (packages/ui): 本計画で新規作成済み

### トラブルシュート最短手順

1. `cd packages/ui && cargo check --quiet` で型/API不整合を先に潰す
2. `cd packages/ui && npx napi build --platform` で addon を再ビルド
3. ルートで `npm run lint` を実行
4. アプリ再起動後に、ズーム波形配置・再生ボタン表示・Deck B load 時の応答性を確認

## ビルド・実行方法

```bash
# @sujay/ui のビルド
cd packages/ui
npx napi build --platform           # debug ビルド
npx napi build --platform --release  # release ビルド

# アプリ起動
cd ../..
npm run start
```

## 参考リンク

- [gpui crates.io](https://crates.io/crates/gpui)
- [Glass-HQ/gpui (standalone fork)](https://github.com/Glass-HQ/gpui)
- [gpui-component (60+ components)](https://github.com/longbridge/gpui-component)
- [wgpu](https://wgpu.rs/)
- [NAPI-RS](https://napi.rs/)
- [Electron getNativeWindowHandle](https://www.electronjs.org/docs/latest/api/browser-window#wingetnahttptivewindowhandle)
