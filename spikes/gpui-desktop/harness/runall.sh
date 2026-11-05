#!/bin/bash
cd ~/.cache/forge-spike-gpui
for n in 1 2 3; do
  python3 -c "import sys; sys.path.insert(0,'.'); import measure_web as m; m.stop_app()"
  python3 web_prep.py
  TAG=web_r$n python3 -u measure_web.py all > results/web_r$n.log 2>&1
  python3 -u stream_web_run.py; TAG=web_r$n python3 -u stream_web.py > results/web_stream_r$n.log 2>&1; cp results/web_stream.json results/web_stream_r$n.json
  python3 -c "import sys; sys.path.insert(0,'.'); import measure_web as m; m.stop_app()"
  TAG=gpui_r$n python3 -u measure_gpui.py > results/gpui_r$n.log 2>&1
  TAG=gpui_r$n python3 -u stream_gpui.py > results/gpui_stream_r$n.log 2>&1
done
echo ALLDONE > results/ALLDONE
