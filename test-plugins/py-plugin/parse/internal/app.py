# app.py
from __future__ import annotations

import json
import tracemalloc

import wit_world
from componentize_py_types import Err
from wit_world.imports import parse_process

peak_mem = 0


def _sample_peak_mem() -> None:
    global peak_mem
    _, peak = tracemalloc.get_traced_memory()
    if peak > peak_mem:
        peak_mem = peak


class WitWorld(wit_world.WitWorld):
    def parse(self, raw_data: list[bytes]) -> list[parse_process.LogEntry]:
        global peak_mem
        peak_mem = 0

        tracemalloc.start()
        try:
            _sample_peak_mem()

            entries: list[parse_process.LogEntry] = []
            _sample_peak_mem()

            for raw_buf in raw_data:
                try:
                    obj = json.loads(raw_buf)
                except Exception as e:
                    raise Err(parse_process.ParseError_InvalidFormat(str(e)))

                if not isinstance(obj, dict):
                    raise Err(
                        parse_process.ParseError_InvalidFormat(
                            "top-level JSON value must be an object"
                        )
                    )
        
                ts = obj.get("ts", "")
                if not isinstance(ts, str):
                    raise Err(
                        parse_process.ParseError_InvalidFormat(
                            'field "ts" must be a string'
                        )
                    )

                level = obj.get("level", "")
                if not isinstance(level, str):
                    raise Err(
                        parse_process.ParseError_InvalidFormat(
                            'field "level" must be a string'
                        )
                    )

                msg = obj.get("msg", "")
                if not isinstance(msg, str):
                    raise Err(
                        parse_process.ParseError_InvalidFormat(
                            'field "msg" must be a string'
                        )
                    )

                att = obj.get("att", {})
                if att is None:
                    att = {}
                if not isinstance(att, dict):
                    raise Err(
                        parse_process.ParseError_InvalidFormat(
                            'field "att" must be an object'
                        )
                    )

                pairs: list[tuple[str, str]] = []
                for key, value in att.items():
                    if not isinstance(key, str):
                        raise Err(
                            parse_process.ParseError_InvalidFormat(
                                'field "att" keys must be strings'
                            )
                        )
                    if not isinstance(value, str):
                        raise Err(
                            parse_process.ParseError_InvalidFormat(
                                'field "att" values must be strings'
                            )
                        )
                    pairs.append((key, value))

                entries.append(
                    parse_process.LogEntry(
                        timestamp=ts,
                        level=parse_process.LogLevel.DEBUG,
                        message=msg,
                        tags=pairs,
                    )
                )

            _sample_peak_mem()
            return entries
        finally:
            tracemalloc.stop()

    def report_usage(self) -> int:
        return peak_mem