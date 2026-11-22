@echo off
rem Starts Wrapped Texture Editor with Vulkan, but without implicit Vulkan layers
rem (OBS game capture hook, Steam overlay, Steam Fossilize, AMD switchable graphics).
set "WGPU_BACKEND=vulkan"
set "VK_LOADER_LAYERS_DISABLE=~implicit~"
start "" "%~dp0Wrapped Texture Editor.exe" %*
