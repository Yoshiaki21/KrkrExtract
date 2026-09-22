# CLAUDE.md

> このファイルはプロジェクトの進行に合わせて Claude Code 自身が更新していく「生きたドキュメント」です。
> 本プロジェクトは既存コード（git clone / fork したもの）です。作業を始める前に、下記「## 初回セットアップ（既存プロジェクト把握）」を必ず実行し、現状のコードベースを読み込んで各章を埋めてください。その後は通常の自己更新ルールに従って育てていきます。

---

## 初回セットアップ（既存プロジェクト把握）※このファイルを読んだら最初に一度だけ実行

このCLAUDE.mdの「## 構成」の各章がまだ空、または実態と大きくズレている場合、以下を行ってください。

1. リポジトリ全体のディレクトリ構造を走査する（Glob/Grepで俯瞰）
2. package.json / *.csproj / requirements.txt / README など設定・説明ファイルを読み、技術スタックを特定する
3. 既存の命名規則・レイヤー構成・設計パターンを、実コードから推測ではなく実例ベースで拾う
4. 「## 構成」の各章（1〜6）を、推測ではなく実際に確認できた情報のみで埋める
   - 特に「3. ディレクトリ構造 / Critical Paths」は、実際に存在する主要ディレクトリ・ファイルを根拠に表を作る
   - 不明な点・複数の流儀が混在している点は、正直に「（要確認）」「（新旧混在）」のように明記する。存在しないものを断定して書かない
5. 埋め終えたら、ユーザーに「初回スキャンが完了し、CLAUDE.mdを埋めました」と一言報告する

この初回セットアップが終わった後は、以下の「自己更新ルール」に従って通常運用に入ってください（再度全体スキャンをやり直す必要はなく、変更差分だけを追記していきます）。

---

## Claude Codeへの指示（自己更新ルール）

あなたはこのプロジェクトで作業するたびに、以下のルールに従って本ファイル（CLAUDE.md）を更新してください。

### 更新すべきタイミング
- 新しいファイル/モジュール/ディレクトリを作成したとき
- 既存のアーキテクチャや設計方針を変更したとき（例: ライブラリの乗り換え、レイヤー構成の変更）
- 「今後も同じ説明をユーザーにさせそうだ」と感じたとき
- ユーザーから明示的に「CLAUDE.mdに書いて」と言われたとき

### 更新時の振る舞い
- 該当する章（下記「## 構成」参照）に、簡潔な箇条書きで追記する。長文の説明文は書かない。
- 「Critical Paths（ファイル所在表）」は特に優先して最新化する。何かを追加/変更したら、まずここを疑う。
- 迷ったら、ユーザーに聞かずにいったん追記し、後で「CLAUDE.mdのここを追記しました」と一言報告する。
- 冗長な履歴は残さない。古い情報は書き換える（変更履歴セクション以外は追記ではなく上書き優先）。
- コード規約・設計判断の「理由」も一言添える（例: 「認証は express-session を採用（JWT不要な単一サーバー構成のため）」）。

### 書かないこと
- 実装の詳細なロジック説明（コード自体やdocstringに書くべきもの）
- 一時的なタスクの進捗（TODOリストや作業ログはここに書かない）
- 憶測・未確定の設計（決まったことだけを書く。初回セットアップ時に不明だった点は「（要確認）」のまま残し、断定で埋めない）

---

## 構成

### 1. プロジェクト概要
- 目的: 吉里吉里（krkr2 / krkrz）エンジンのゲームに DLL を注入し、xp3 アーカイブの展開・再パック、各種形式（TLG / PSB / PBD / AMV / 暗号化テキスト等）のデコード、Universal Dumper / Universal Patch 生成を行うツール
- 想定ユーザー: ビジュアルノベルの解析・翻訳パッチ制作者（上流 xmoeproject は README で「メンテナンス終了」と明言。後継は KrkrzExtract）
- ライセンス: GPLv3

### 2. 技術スタック
- 言語/フレームワーク: C++（Core は `stdcpp20`、UI.Lite は `stdcpp17`）/ C#（.NET Framework 4.8, WinForms）/ Win32 API・NT Native API
- ビルド: Visual Studio ソリューション `KrkrExtract/KrkrExtract.sln`。PlatformToolset は `v143`（VS2022）、PatchLoader のみ `v142`。README は「vs2019 / Win10 SDK 10.0.17763.0」と記載（新旧混在・要確認）
- ターゲット: **x86 (Win32) のみ**。64bit 版ソースは削除済み（README）。Windows 専用で Linux ではビルド不可
- DB: SQLite（`KrkrExtract.Core/sqlite3.c` 同梱）— Universal Patch 用 `KrkrExtract.db` の生成/読み込み
- 主要ライブラリ:
  - gRPC + protobuf（Core ⇔ UI 間 RPC。ヘッダは `3rdParts/include`、libは `3rdParts/lib32/{debug,release}` を参照）
  - capstone（逆アセンブル）、jsoncpp、zlib、libpng、lz4、xxhash、md5/sha1、magic_enum、argparse（いずれもソリューション内 or Core 内に同梱）
  - NativeLib（phnt ベースの NT API ラッパ、`Ps::` `Nt::` 等の名前空間。静的ライブラリ）
  - Lite（C#）: Costura.Fody（単一exe化）、Newtonsoft.Json 13.0.1
  - `krkrz/` は吉里吉里Z 本体ソース（`tvpwin32` 等。デバッグ/ASan 用ターゲット）

### 3. ディレクトリ構造 / Critical Paths（最重要）
パスはリポジトリルート基準。

| やりたいこと | 場所 |
|---|---|
| ソリューションを開く / プロジェクト構成を見る | `KrkrExtract/KrkrExtract.sln` |
| 注入される本体ロジック（DLL）を触る | `KrkrExtract/KrkrExtract.Core/`（エントリ `Main.cpp` の `DllMain`、中核クラス `KrkrExtractCore` は `KrkrExtract.h` / `KrkrExtract.cpp`） |
| xp3 の解析 | `KrkrExtract.Core/XP3Parser.{h,cpp}`、`Xp3*NodeValidator*.cpp`（チャンク形式ごとの検証器） |
| 形式別の展開処理を追加/修正 | `KrkrExtract.Core/*Unpacker.cpp`（Psb/Tlg/Png/Pbd/Amv/Text/File）＋ `*Decoder.cpp` / `*Decode.cpp` |
| PSB / PIMG（PSB形式のレイヤー画像コンテナ）の展開 | `KrkrExtract.Core/PsbWorker.cpp`（`DumpPsbTjs2` が入口、`dumpPimg` が画像書き出し、`dump()` がツリーのTJS出力）、JSON化は `PsbDecompilerJson.cpp`。スタンドアロン版は `ToolSource/EMoteDumper/` |
| PIMG 単体の解凍＋差分合成ツール（Python・標準ライブラリのみ） | `ToolSource/pimgext/`（仕様は `SPEC.md`） |
| 再パック（krkr2 向け） | `KrkrExtract.Core/KrkrPacker.cpp` |
| Universal Dumper（krkrz 限定） | `KrkrExtract.Core/TaskUniversalDumper.cpp`、`KrkrDumper.cpp` |
| Universal Patch 生成 / パッチDLL本体 | 生成: `KrkrExtract.Core/KrkrUniversalPatch.cpp` / DLL: `KrkrExtract/KrkrzUniversalPatch/`（出力名 `KrkrUniversalPatch.dll`） |
| フック・保護回避 | `KrkrExtract.Core/Hook.cpp`、`HookBypass.cpp`、`KrkrHookBypass.cpp`、`EptHook.cpp`、`HardwareBreakpoint.h` |
| 実行モード（LOCAL/REMOTE/MIXED）と UI 通知 | `KrkrExtract.Core/ServerStub.cpp`、`ClientImpl.cpp` |
| RPC 定義を変更 | `KrkrExtract/KrkrExtract.Shared/client.proto`（`KrCoreApi`）・`server.proto`（`KrConnectionApi`）＋個別 `*.proto` → `gen.bat` / `gen.sh` で `*.pb.*` を再生成 |
| Core/UI 共通ヘッダ | `KrkrExtract/KrkrExtract.Shared/`（`RpcDefine.h`, `Prototype.h`, `tp_stub.*` 等） |
| ネイティブ UI（ダイアログ） | `KrkrExtract/KrkrExtract.UI.Lite/`（DLL、`KrCreateWindow` をエクスポート。`UIViewer.h`） |
| ランチャー（exe をドロップしてDLL注入） | `KrkrExtract/KrkrExtract.Lite/`（C#。`LoaderHelper.CreateProcessWithDll` で `KrkrExtract.Core.dll` を注入） |
| パッチローダー | `KrkrExtract/PatchLoader/`（sln 未登録・要確認） |
| NT API ラッパ | `KrkrExtract/NativeLib/`（`my.h`） |
| ビルド生成物の掃除 | `KrkrExtract/cleanup.py` |
| 単体の補助ツール（別ソリューション） | `ToolSource/`（tjs2 Compiler/Disassembler、TLGDecoder、PbdDecoder、EMote 系、Krkr2Packer、M2Packer 等） |
| 吉里吉里向けSDK/サンプル | `SDK/`（KrkrFile, KrkrFilePacker, alphamovie） |
| デバッガ用スクリプト | `scripts/ida/`、`scripts/r2/`、`Script/break_onName.txt` |
| 公式サイト（GitHub Pages） | `docs/`（静的HTML） |

### 4. 設計方針・規約
- 命名: 型・関数・メソッドは PascalCase、メンバ変数は `m_` + PascalCase（例: `m_RunMode`）、Win32 型（`NTSTATUS`, `PCWSTR`, `BOOL`）を多用。戻り値は基本 `NTSTATUS` で `NT_FAILED` 判定（Windows カーネル流儀に合わせるため）
- Core はシングルトン（`KrkrExtractCore::GetInstance()`）。DLL は `DllMain` → `Initialize` で起動
- エクスポートは `#pragma comment(linker, "/EXPORT:Name=_Name@N")` で stdcall 修飾名を明示（x86 限定のため）
- プロセス構成: Lite(C#) がゲームを起動して Core.dll を注入 → Core が UI.Lite のダイアログを表示。Core⇔UI は gRPC（REMOTE/MIXED モード）または同一プロセス直接呼び出し（LOCAL モード）
- 1ファイル1機能（`XxxUnpacker.cpp`, `XxxDecoder.cpp`）で形式ごとに分割
- サードパーティは vendoring（リポジトリに直接同梱）。ファイル名の流儀は新旧混在（`XP3Parser` / `Xp3Parser.ixx` 等）
- 自動テストは存在しない

### 5. よく使うコマンド
```bat
:: Windows + Visual Studio 前提（README: Release ビルドのみサポート）
:: ※msbuild コマンド自体は README 未記載。sln の構成名から組み立てたもの（要確認）
msbuild KrkrExtract\KrkrExtract.sln /p:Configuration=Release /p:Platform=x86

:: protobuf / gRPC コード再生成（KrkrExtract.Shared で実行）
gen.bat
```
```bash
# ビルド生成物の削除
python3 KrkrExtract/cleanup.py

# PIMG の解凍＋差分合成（Linux でも動く。sample.pimg の横に sample/raw, sample/composite を作る）
python3 ToolSource/pimgext/pimgext.py sample.pimg
```
- ASan 付きデバッグビルド手順は README（`img/step*.png`）参照。ASan 対応の吉里吉里本体が必要

### 6. 既知の制約・注意点
- **このリポジトリだけではビルドできない可能性が高い**: Core がリンクする `3rdParts/lib32/{debug,release}/*.lib`（grpc/absl/protobuf/ssl 等）がリポジトリに存在しない。`KrkrExtract/grpc` サブモジュールも未チェックアウト（要確認: 自前で grpc を x86 ビルドする必要あり）
- ビルド済みバイナリ（`KrkrUniversalPatch.dll`, `PatchLoader.exe`, `tvpwin64.exe`, `libpng16.lib`, `protoc.exe` 等）がソースツリーに直接コミットされている。`ToolSource/` 下にも `Release/`・`.tlog` 等の中間物が混入
- `*.pb.cc/*.pb.h` は生成物だがコミット済み。`.proto` を変えたら必ず再生成してコミット
- 保護（パッカー/難読化）された実行ファイルは非対応（README 方針）
- Universal Patch は krkrz 専用（krkr2 は BCB ビルドのため解析困難）。krkr2 は UI のパック機能を使う
- `KrkrExtract/package.json`（chromehtml2pdf）は用途不明（要確認）
- Core の Debug 構成は `libprotobufd.lib` 等 debug 版 lib を要求

### 7. 変更履歴（任意）
（このプロジェクトに手を入れ始めてからの、大きな方針転換のみ日付と概要を1行で）

-

---

**運用メモ（ユーザー向け）**
- 初回スキャンの結果は完璧ではありません。特に「3. Critical Paths」と「4. 設計方針」は、実際に何か修正を依頼したときの挙動を見ながら、都度手直ししてください。
- ある程度育ってきたら、章ごとに `docs/` 配下へ分割してこのファイルからリンクする構成に移行することを検討してください（tududiのCLAUDE.mdのような形）。
- 定期的に「CLAUDE.mdを見直して、実態と合っていない箇所を修正して」とClaude Codeに依頼すると、陳腐化を防げます。
