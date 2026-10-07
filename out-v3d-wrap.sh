#!/usr/bin/env bash
export HH_SAN=0
export HH_FEATURES=osal-linux,gpu-v3d,shaderc-online
exec tools/hostharness/run.sh all out-v3d rust:. driver dev
