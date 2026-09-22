@echo off
REM Python 3.12を優先的に使用
set PATH=D:\stable-diffusion\Python\Python312;D:\stable-diffusion\Python\Python312\Scripts;%PATH%
REM === Git PATH設定（追加）===
set PATH=C:\Program Files\Git\bin;%PATH%

REM Pythonバージョン確認
echo ====================================
echo Python 3.12 環境を起動しました
python --version
echo ====================================
echo.

REM 新しいコマンドプロンプトを起動（PATH設定を引き継ぐ）
cmd /k