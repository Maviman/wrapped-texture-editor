@echo off
rem Starts Wrapped Texture Editor with the DirectX 12 renderer instead of Vulkan.
set "WGPU_BACKEND=dx12"
start "" "%~dp0Wrapped Texture Editor.exe" %*
