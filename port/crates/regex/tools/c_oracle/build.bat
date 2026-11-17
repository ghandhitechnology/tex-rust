@echo off
rem Build the secondary C oracle with MSVC.  Usage: build.bat <texlive-source>\texk\web2c\pdftexdir <outdir>
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
cl /nologo /O2 /W0 /I "%~1" /Fe"%~2\runc.exe" /Fo"%~2\\" "%~dp0runc.c" "%~1\regex\regex.c" /link /STACK:5242880
