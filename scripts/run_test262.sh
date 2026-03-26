#!/usr/bin/env bash
# Run test262-inspired JavaScript tests to see what tish can handle.
# Usage: ./scripts/run_test262.sh [--verbose] [--filter PATTERN]
#
# Options:
#   --verbose   Show full output for each test
#   --filter    Only run tests matching PATTERN (e.g., "expressions" or "Array")

set -e
cd "$(dirname "$0")/.."

TISH_BIN="cargo run -p tishlang-q --features full --"
TEST262_DIR="tests/test262"
HARNESS="$TEST262_DIR/harness.js"

verbose=false
filter=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --verbose|-v) verbose=true; shift ;;
        --filter|-f) filter="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ ! -f "$HARNESS" ]]; then
    echo "Error: harness.js not found at $HARNESS"
    exit 1
fi

total=0
passed=0
failed=0
failed_tests=()

echo "═══════════════════════════════════════════════════════════════════"
echo "  Tish Test262 Suite"
echo "═══════════════════════════════════════════════════════════════════"
echo ""

# Find all .js test files (excluding harness.js)
while IFS= read -r -d '' test_file; do
    [[ "$test_file" == "$HARNESS" ]] && continue
    
    # Apply filter if specified
    if [[ -n "$filter" ]] && [[ "$test_file" != *"$filter"* ]]; then
        continue
    fi
    
    rel_path="${test_file#$TEST262_DIR/}"
    total=$((total + 1))
    
    # Create temp file with harness + test
    tmp_file=$(mktemp /tmp/tish_test262_XXXXXX.js)
    cat "$HARNESS" "$test_file" > "$tmp_file"
    
    # Run the test
    if $verbose; then
        echo "─────────────────────────────────────────────────────────────────"
        echo "▶ $rel_path"
        echo "─────────────────────────────────────────────────────────────────"
        if output=$($TISH_BIN run "$tmp_file" 2>&1); then
            echo "$output"
            if echo "$output" | grep -q "FAIL:"; then
                failed=$((failed + 1))
                failed_tests+=("$rel_path")
                echo "  ❌ ASSERTIONS FAILED"
            else
                passed=$((passed + 1))
                echo "  ✓ PASSED"
            fi
        else
            echo "$output"
            failed=$((failed + 1))
            failed_tests+=("$rel_path")
            echo "  ❌ RUNTIME ERROR"
        fi
        echo ""
    else
        # Quiet mode - just show pass/fail
        if output=$($TISH_BIN run "$tmp_file" 2>&1); then
            if echo "$output" | grep -q "FAIL:"; then
                echo "❌ $rel_path"
                failed=$((failed + 1))
                failed_tests+=("$rel_path")
            else
                echo "✓ $rel_path"
                passed=$((passed + 1))
            fi
        else
            echo "❌ $rel_path (runtime error)"
            failed=$((failed + 1))
            failed_tests+=("$rel_path")
        fi
    fi
    
    rm -f "$tmp_file"
done < <(find "$TEST262_DIR" -name "*.js" -print0 | sort -z)

echo ""
echo "═══════════════════════════════════════════════════════════════════"
echo "  Results: $passed/$total passed"
echo "═══════════════════════════════════════════════════════════════════"

if [[ $failed -gt 0 ]]; then
    echo ""
    echo "Failed tests ($failed):"
    for t in "${failed_tests[@]}"; do
        echo "  - $t"
    done
    echo ""
fi

# Exit with error if any tests failed
[[ $failed -eq 0 ]]
