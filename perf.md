════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress            10012       26    10011    10010       41       23       36       67     24419%
object_stress             989       40      103      355       34       20       29       66      2908%
benchmark_granular        881       30       98      327       35       21       30       69      2517%
new_features_perf         482       28       63      182       35       19       30       64      1377%
string_methods_perf        30        9       10       22       28       13       23        8       107%
objects_perf               22        9       10       20       28       13       24        7        78%
array_methods_perf         18        8        9       20       28       12       23        9        64%
nested_complex             11        8        8       16       28       13       24        8        39%
template_literals          10        9        9       15       28       12       24        7        35%
arrays                     10        9        9       15       28       13       25        7        35%
objects                    10        8        9       15       28       12       24        7        35%
math                       10        8        9       16       28       12       23        7        35%
higher_order_methods       10        8        8       15       28       13       24        7        35%
const                      10        9        8       15       28       13       24        7        35%
array_methods              10        9        8       15       28       12       23        7        35%
nested_loops               10        9        9       16       29       13       23        7        34%
mutation                   10        9        8       16       29       12       24        7        34%
rest_params                 9        8        8       15       27       13       23        7        33%
compound_assign             9        8        8       15       28       12       23        7        32%
builtins                    9        9        9       16       28       13       24        7        32%
break_continue              9        9        8       15       28       12       24        7        32%
void                        9        8        9       15       28       12       24        7        32%
uri                         9        9        8       15       28       12       23        7        32%
types                       9        8        8       15       28       13       24        7        32%
typeof                      9        9        9       15       28       12       23        7        32%
try_catch                   9        8        8       15       28       13       23        7        32%
switch                      9        8        8       15       28       13       24        7        32%
string_methods              9        8        9       15       28       13       24        7        32%
strict_equality             9        9        8       15       28       12       24        7        32%
space_indent                9        8        8       15       28       12       23        7        32%
scopes                      9        8        8       15       28       13       24        7        32%
optional_chaining           9        8        8       15       28       12       23        7        32%
optional_braces_braced        9        8        9       15       28       12       23        7        32%
optional_braces             9        8        8       15       28       12       23        7        32%
length                      9        8        9       15       28       12       24        7        32%
json                        9        8        8       15       28       14       23        7        32%
inc_dec                     9        8        8       15       28       12       24        7        32%
in_op                       9        8        8       15       28       12       23        7        32%
for_of                      9        8        8       15       28       12       23        7        32%
fn_any                      9        9        8       15       28       12       23        7        32%
exponentiation              9        8        8       15       28       13       23        7        32%
do_while                    9        9        9       15       28       13       23        7        32%
conditional                 9        9        9       16       28       14       25        8        32%
arrow_functions             9        9        9       16       29       14       26        7        31%
bitwise                     9        8        8       15       31       13       23        7        29%
tab_indent                  8        8        8       15       28       13       24        7        28%
object_methods              9        9        8       15       33       13       29        7        27%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                   12804      486    10638    11543     1359      624     1145      572       942%


════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1413       26      180      559       41       23       36       66      3446%
object_stress            1000       40      105      364       35       23       32       67      2857%
benchmark_granular        853       30       96      315       36       22       31       71      2369%
new_features_perf         491       28       62      184       36       20       31       64      1363%
objects_perf               23        9       10       20       29       14       26        8        79%
array_methods_perf         18        9       10       20       29       15       26       10        62%
length                     12        9        8       15       28       13       24        7        42%
strict_equality            11        9        9       15       28       13       24        7        39%
space_indent               11       10       10       16       28       14       25        8        39%
scopes                     11        9        9       16       28       14       26        8        39%
void                       11        9        9       16       29       14       25        7        37%
optional_braces_braced       11        9        9       16       29       13       25        8        37%
nested_complex             11        9        9       15       29       13       24        7        37%
optional_chaining          11       12       11       19       30       15       26        8        36%
compound_assign            10        9        8       15       28       13       24        7        35%
uri                        10        8        9       15       28       13       24        7        35%
types                      10        8        9       15       28       13       24        7        35%
try_catch                  10        9        9       16       28       13       25        7        35%
template_literals          10        9        9       16       28       13       25        8        35%
tab_indent                 10        9        9       16       28       14       25        7        35%
switch                     10        9        9       16       28       13       24        7        35%
string_methods_perf        10       10       10       16       28       15       26        9        35%
string_methods             10        9        9       15       28       13       25        8        35%
rest_params                10       10        9       16       28       13       24       10        35%
arrays                     10        9        8       15       28       13       26        7        35%
math                       10        8        9       15       28       13       24        7        35%
in_op                      10        8        9       16       28       13       24        7        35%
higher_order_methods       10        9        9       15       28       13       24        7        35%
for_of                     10        9        8       15       28       13       24        7        35%
fn_any                     10        9        9       15       28       13       24        7        35%
const                      10        9        9       15       28       13       24        7        35%
builtins                   10        9        9       15       29       13       24        7        34%
typeof                     10        9        9       15       29       14       25        7        34%
arrow_functions            10        9        9       15       29       13       24        7        34%
objects                    10        9       10       16       29       13       26        7        34%
json                       10        9        8       15       29       13       25        8        34%
exponentiation             10        9        9       15       29       13       24        8        34%
do_while                   10        9        9       15       29       14       25        7        34%
array_methods              10       11        9       15       29       13       24        7        34%
bitwise                     9        8        9       15       28       13       24        7        32%
mutation                    9        9        9       15       28       13       24        7        32%
inc_dec                     9        8        9       15       28       13       24        7        32%
conditional                 9        8        8       15       28       13       24        8        32%
break_continue              9        9        9       15       29       13       24        7        31%
optional_braces             9        9        9       16       29       14       30        9        31%
nested_loops                9        8        8       15       29       13       25        7        31%
object_methods             10        9        9       15       34       13       30        7        29%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4210      511      830     2094     1376      661     1199      589       305%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | wasi=wasmtime

─────────────────────────────────────────



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1413       28      179      563       41       24       38       67      3446%
object_stress             998       40      106      357       35       21       30       67      2851%
benchmark_granular        851       30       96      316       35       22       30       70      2431%
new_features_perf         487       28       62      184       35       20       31       64      1391%
objects_perf               23        9        9       19       29       14       24        7        79%
array_methods_perf         20        9       10       20       29       13       25        9        68%
arrow_functions            11        9        9       16       28       14       25        8        39%
nested_complex             11        9        9       15       28       13       24        8        39%
compound_assign            10        9        8       15       28       13       24        7        35%
builtins                   10        9        9       15       28       13       24        7        35%
break_continue             10        9        9       15       28       13       24        7        35%
types                      10        9        9       15       28       13       24        7        35%
strict_equality            10        8        9       15       28       13       25        7        35%
space_indent               10        9        8       15       28       13       24        7        35%
optional_chaining          10        9        9       15       28       13       24        7        35%
length                     10        8        9       15       28       13       24        7        35%
template_literals          10        9        9       15       29       13       24        8        34%
string_methods_perf        10        9        8       14       29       14       25        8        34%
string_methods             10        9        8       15       29       14       24        7        34%
rest_params                10        9        9       15       29       13       24        7        34%
optional_braces            10        9        9       15       29       14       24        7        34%
arrays                     10        9        9       16       29       13       24        8        34%
objects                    10        9        9       15       29       13       31        7        34%
nested_loops               10        8        8       15       29       13       24        7        34%
mutation                   10        9        9       15       29       13       24        7        34%
math                       10        9        8       15       29       13       24        7        34%
json                       10        8        9       15       29       13       25        7        34%
higher_order_methods       10        9        9       15       29       13       24        7        34%
for_of                     10        9        8       15       29       13       24        7        34%
fn_any                     10        9        8       15       29       13       24        7        34%
const                      10        9        9       15       29       13       24        7        34%
array_methods              10        9        9       15       29       13       24        7        34%
tab_indent                 10        9        8       17       30       13       24        7        33%
optional_braces_braced        9        9        9       15       27       13       24        7        33%
bitwise                     9        9        9       15       28       13       25        7        32%
void                        9        9        8       15       28       13       24        7        32%
switch                      9        9        9       15       28       13       24        7        32%
scopes                      9        9        9       15       28       13       24        7        32%
in_op                       9        9        9       15       28       13       24        7        32%
exponentiation              9        8        8       15       28       14       24        7        32%
do_while                    9        9        9       15       28       13       23        7        32%
conditional                 9        9        8       15       28       13       24        7        32%
uri                         9        9        8       15       29       13       24        7        31%
typeof                      9        9        9       15       29       13       24        7        31%
try_catch                   9        9        8       15       29       13       24        7        31%
inc_dec                     9        9        8       16       29       13       24        7        31%
object_methods             10        9        9       15       34       13       30        7        29%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4191      508      816     2078     1379      652     1179      576       303%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | wasi=wasmtime

─────────────────────────────────────────
Done.



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1488       31      170      591       42       24       36       66      3542%
object_stress            1043       41      108      357       35       21       30       67      2980%
benchmark_granular        907       32      100      319       36       22       30       70      2519%
new_features_perf         514       29       63      183       36       19       31       64      1427%
objects_perf               23        9       10       20       28       14       25        8        82%
array_methods_perf         19        9       10       20       29       13       25       12        65%
nested_complex             11        9        9       16       28       13       24        7        39%
in_op                      10        9        8       15       27       13       24        7        37%
for_of                     10        8        8       15       27       13       24        7        37%
builtins                   10        9        8       15       28       13       24        7        35%
break_continue             10        8        8       15       28       13       24        7        35%
uri                        10        9        9       15       28       13       24        7        35%
optional_braces_braced       10        9        9       15       28       12       24        7        35%
objects                    10        9        9       15       28       13       24        7        35%
math                       10        9        8       15       28       13       24        7        35%
json                       10        8        9       15       28       13       24        7        35%
higher_order_methods       10        9        9       16       28       13       24        7        35%
conditional                10        9        9       15       28       12       24        7        35%
compound_assign            10        9        9       15       29       13       24        7        34%
void                       10        9        9       16       29       14       25        8        34%
types                      10        8        9       17       29       14       23        7        34%
template_literals          10        9        9       15       29       13       24        7        34%
tab_indent                 10        9        9       15       29       13       24        7        34%
arrow_functions            10        9        8       15       29       13       24        7        34%
switch                     10        9        8       15       29       12       24        7        34%
string_methods_perf        10       10        9       15       29       14       24        8        34%
string_methods             10        9        9       14       29       13       25        7        34%
scopes                     10        9        9       15       29       13       24        7        34%
rest_params                10        9        9       15       29       13       24        7        34%
length                     10        9        9       15       29       13       24        7        34%
do_while                   10        9        9       15       29       13       24        7        34%
array_methods              10        9        9       15       29       14       25        7        34%
typeof                      9        8        9       15       28       12       24        7        32%
try_catch                   9        8        8       15       28       13       24        7        32%
strict_equality             9        9        9       15       28       13       24        7        32%
optional_chaining           9        9        9       15       28       13       24        7        32%
inc_dec                     9        9        8       15       28       13       24        7        32%
exponentiation              9        8        9       15       28       13       24        7        32%
const                       9        9        9       15       28       13       24        7        32%
bitwise                     9        9        9       15       29       13       24        7        31%
space_indent                9        9        8       15       29       13       25        7        31%
optional_braces             9        9        8       15       29       13       24        7        31%
arrays                      9        9        9       15       29       13       24        7        31%
nested_loops                9        9        8       15       29       13       24       10        31%
mutation                    9        9        8       15       29       13       24        7        31%
fn_any                      9        9        9       15       29       13       24        7        31%
object_methods              9        9        9       15       34       13       30        7        26%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4390      514      817     2109     1379      646     1170      579       318%



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1405       27      186      563       42       24       36       67      3345%
array_methods_perf         19        9       10       20       29       14       25        9        65%
arrays                     10        9        9       15       29       13       25        7        34%
array_methods              10        9        9       15       29       13       24        7        34%



════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1413       26      175      562       42       24       36       66      3364%
object_stress            1000       41      104      360       35       21       30       67      2857%
benchmark_granular        851       30       97      317       35       23       31       71      2431%
new_features_perf         487       28       62      185       36       19       31       64      1352%
objects_perf               23        9       10       20       29       14       24        8        79%
array_methods_perf         20       10       11       21       30       15       26       10        66%
nested_complex             11        8        9       15       29       13       25        8        37%
higher_order_methods       11        9        8       16       29       14       24        7        37%
array_methods              11       10       10       16       29       14       26        8        37%
builtins                   10        9        9       16       28       13       25        8        35%
types                      10        9        9       15       28       13       24        7        35%
template_literals          10        8        9       15       28       13       24        7        35%
switch                     10        9        9       15       28       13       24        7        35%
string_methods             10        9        9       15       28       13       24        7        35%
strict_equality            10        8        9       15       28       13       24        7        35%
scopes                     10        9        9       15       28       13       24        7        35%
inc_dec                    10        9        9       15       28       13       24        7        35%
for_of                     10        8        8       15       28       13       24        8        35%
do_while                   10        8        9       15       28       13       24        7        35%
compound_assign            10        9        9       15       29       13       25        7        34%
void                       10        9        9       15       29       13       24        7        34%
string_methods_perf        10       10        9       15       29       14       24        8        34%
rest_params                10        9        9       15       29       13       24        7        34%
optional_braces_braced       10        8        8       15       29       13       24        7        34%
objects                    10        9        9       15       29       13       24        7        34%
nested_loops               10        8        9       15       29       13       24        7        34%
mutation                   10        9        8       15       29       13       24        7        34%
length                     10        9        9       15       29       13       24        7        34%
json                       10        9        9       15       29       13       24        7        34%
fn_any                     10        9        9       17       29       13       24        7        34%
exponentiation             10        9        9       15       29       13       24        7        34%
const                      10        9        9       15       29       13       25        7        34%
uri                         9        9        8       15       27       13       24        7        33%
arrow_functions            10        9        9       16       30       14       25        7        33%
break_continue              9        9        9       16       28       14       25        7        32%
try_catch                   9        9        9       15       28       13       25        7        32%
space_indent                9        9        8       15       28       13       24        8        32%
optional_chaining           9        9        8       15       28       13       24        7        32%
optional_braces             9        9        9       15       28       13       24        7        32%
bitwise                     9        9        9       15       29       13       24        7        31%
typeof                      9        9        9       15       29       13       26        7        31%
tab_indent                  9        9        8       15       29       13       24        7        31%
arrays                      9        9        8       15       29       14       25        8        31%
math                        9        8        9       15       29       13       25        7        31%
in_op                       9        9       10       16       29       13       24        7        31%
conditional                 9        9        8       15       29       13       24        8        31%
object_methods             10        9        9       15       34       13       30        8        29%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4194      507      820     2088     1385      655     1181      582       302%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | wasi=wasmtime

─────────────────────────────────────────






════════════════════════════════════════════════════════════════════════════════════════════════════════════════
                                           PERFORMANCE SUMMARY
                                    (sorted by Tish(run)/Node ratio, slowest first)
════════════════════════════════════════════════════════════════════════════════════════════════════════════════

Test                      run     rust cranelift     llvm     wasi     Node      Bun     Deno  QuickJS  run/Node%
──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
array_stress             1412       26      177      178      560       42       24       36       66      3361%
object_stress            1003       39      104      104      360       35       21       31       69      2865%
benchmark_granular        847       29       95       96      312       35       22       32       70      2420%
new_features_perf         487       26       62       62      184       37       20       31       65      1316%
objects_perf               23        9       10       10       20       29       14       25        8        79%
array_methods_perf         19        9       10       10       20       29       14       25        9        65%
tab_indent                 12       14        9        8       15       28       14       24        8        42%
nested_complex             11        9        9        9       16       29       14       25        8        37%
fn_any                     11        9        9        9       15       29       13       25        7        37%
uri                        10        9        9        9       15       28       13       25        8        35%
typeof                     10        9        9        9       15       28       13       25        7        35%
template_literals          10        9        8        8       15       28       13       25        7        35%
arrow_functions            10        9        9        9       15       28       13       25        8        35%
strict_equality            10        9        9        9       15       28       13       25        7        35%
rest_params                10        9        9        9       15       28       13       25        7        35%
arrays                     10        9        9        9       15       28       13       24        8        35%
in_op                      10        9        9        9       16       28       14       25        7        35%
compound_assign            10        9        9        9       16       29       13       24        7        34%
builtins                   10        9        9        9       15       29       13       25        7        34%
break_continue             10        9        8        9       15       29       13       24        8        34%
bitwise                    10        9        9        8       16       29       13       24        7        34%
types                      10        9        9        9       16       29       14       31        9        34%
try_catch                  10        9        9        9       15       29       13       24        7        34%
switch                     10        8        9        9       16       29       13       24        7        34%
string_methods_perf        10        9        9        9       15       29       14       25        9        34%
string_methods             10        9        9        9       15       29       15       27        8        34%
space_indent               10        9        9        9       15       29       13       25        7        34%
scopes                     10        9        9        9       15       29       14       24        7        34%
optional_braces_braced       10        9        9        9       16       29       13       27        7        34%
objects                    10        9        9        9       15       29       13       25        7        34%
nested_loops               10        9        9        9       15       29       13       24        7        34%
mutation                   10        9        9        8       15       29       13       24        7        34%
math                       10        8        9        9       15       29       13       24        7        34%
length                     10        8        9        8       14       29       13       25        7        34%
json                       10        9        9        8       16       29       13       24        7        34%
inc_dec                    10        9        9        9       15       29       14       25        7        34%
higher_order_methods       10        9        9        9       16       29       14       24        8        34%
for_of                     10        9        9        9       15       29       13       24        8        34%
exponentiation             10        9        9        9       15       29       13       24        7        34%
do_while                   10        9        9        9       15       29       13       24        7        34%
const                      10        9        9        9       15       29       14       25        7        34%
array_methods              10        9        9        9       15       29       14       25        8        34%
optional_chaining           9        9        8        9       15       28       14       26        7        32%
optional_braces             9        9        9        9       15       28       13       24        7        32%
void                        9        8        9        9       15       29       13       24        7        31%
conditional                 9        9        9        9       16       29       13       25        8        31%
object_methods             10        9        9        9       15       34       14       30        7        29%

──────────────────── ────── ────── ────── ────── ────── ────── ────── ────── ────── ──────────
TOTAL                    4201      508      824      823     2080     1390      662     1203      589       302%

Legend: Green = <150% | Yellow = 200-500% | Red = >500%
        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime





./scripts/run_object_stress_profile.sh; \
./scripts/run_benchmark_granular_profile.sh; \

./scripts/run_object_stress_profile.sh --instrument; \
TISH_PROFILE=1 cargo run -p tish --features "full,profile" -- run tests/core/benchmark_granular_04_nested_fn.tish