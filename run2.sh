#!/bin/bash
cd /home/orion/connected-work/01a19dd0-c5a9-77db-86d3-9f9c9a1c0819
export ZEPHYR_WS=/home/orion/connected-deps/zephyr-ws CARGO_TARGET_DIR=/home/orion/connected-work/01a19dd0-c5a9-77db-86d3-9f9c9a1c0819/target
unset MESA_V3D_TREE DEQP_GLES3_TEST_DIR
date +%s > gatestart2.txt
time tools/check.sh > gate.log 2>&1
echo "EXIT $?" >> gate.log
echo FINISHED >> gate.log
