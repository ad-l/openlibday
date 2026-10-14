@echo off
pythonw "%~dp0map_editor.py"
if errorlevel 1 (
    echo Unable to start the editor. Install Python with Tcl/Tk, build the libday CLI with cargo build, and set LIBDAY_ROOT.
    pause
)
