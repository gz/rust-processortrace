[![Build Status](https://travis-ci.org/gz/processor-trace.svg?branch=master)](https://travis-ci.org/gz/processor-trace)

# Processor Trace

Use Intel PT to trace your rust program (work in progess).


python decode.py trace_fn deadbeef > trace_fn.sideband
./sptdecode -s ./trace_fn.sideband -p trace_fn.ptdump
