#!/usr/bin/env bash
# SWE-Grep Benchmark Script
# Usage: ./scripts/benchmark.sh [iterations]
# Default: 1000 iterations

set -e

ITERATIONS=${1:-1000}
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
BINARY="$PROJECT_ROOT/target/release/swe-grep"
OUTPUT_FILE="$PROJECT_ROOT/benchmark-results.json"
TEMP_DIR=$(mktemp -d)

trap "rm -rf $TEMP_DIR" EXIT

echo "╔════════════════════════════════════════════════════════════╗"
echo "║           SWE-Grep Benchmark Suite                         ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

# Check if binary exists
if [[ ! -f "$BINARY" ]]; then
    echo "Building release binary..."
    cd "$PROJECT_ROOT" && cargo build --release 2>/dev/null
fi

echo "Binary: $BINARY"
echo "Iterations: $ITERATIONS"
echo "Output: $OUTPUT_FILE"
echo ""

# Initialize result files
for test in simple_symbol wildcard_search with_body code_only exclude_pattern concept_search batch_search multi_language; do
    echo "0" > "$TEMP_DIR/${test}_sum"
    echo "999999" > "$TEMP_DIR/${test}_min"
    echo "0" > "$TEMP_DIR/${test}_max"
    echo "0" > "$TEMP_DIR/${test}_count"
done

run_test() {
    local name=$1
    shift
    local latency
    latency=$($BINARY search "$@" 2>/dev/null | jq -r '.summary.latency_ms' 2>/dev/null || echo "0")

    if [[ "$latency" =~ ^[0-9]+$ ]] && [[ "$latency" != "0" ]]; then
        local sum=$(cat "$TEMP_DIR/${name}_sum")
        local min=$(cat "$TEMP_DIR/${name}_min")
        local max=$(cat "$TEMP_DIR/${name}_max")
        local count=$(cat "$TEMP_DIR/${name}_count")

        echo $((sum + latency)) > "$TEMP_DIR/${name}_sum"
        echo $((count + 1)) > "$TEMP_DIR/${name}_count"

        if ((latency < min)); then
            echo "$latency" > "$TEMP_DIR/${name}_min"
        fi
        if ((latency > max)); then
            echo "$latency" > "$TEMP_DIR/${name}_max"
        fi
    fi
}

echo "Running $ITERATIONS iterations..."
echo ""

START_TIME=$(date +%s)

for ((i=1; i<=ITERATIONS; i++)); do
    # Progress every 50 iterations
    if ((i % 50 == 0)) || ((i == 1)); then
        printf "\rProgress: %d/%d (%.1f%%)" "$i" "$ITERATIONS" "$(echo "scale=1; $i * 100 / $ITERATIONS" | bc)"
    fi

    run_test "simple_symbol" --agent --concise --symbol SearchConfig --language rust --path "$PROJECT_ROOT"
    run_test "wildcard_search" --agent --concise --symbol '*Config*' --wildcard --language rust --path "$PROJECT_ROOT"
    run_test "with_body" --agent --concise --symbol SearchConfig --language rust --body --path "$PROJECT_ROOT"
    run_test "code_only" --agent --concise --symbol concept --code-only --path "$PROJECT_ROOT"
    run_test "exclude_pattern" --agent --concise --symbol ignore_case --exclude '*.md' --path "$PROJECT_ROOT"
    run_test "concept_search" --agent --concise --concept 'cache warming' --path "$PROJECT_ROOT"
    run_test "batch_search" --agent --concise --symbols 'SearchConfig,PatternMode,SearchEngine' --language rust --path "$PROJECT_ROOT"
    run_test "multi_language" --agent --concise --symbol main --path "$PROJECT_ROOT"
done

END_TIME=$(date +%s)
TOTAL_TIME=$((END_TIME - START_TIME))

echo ""
echo ""
echo "╔════════════════════════════════════════════════════════════╗"
echo "║                    Benchmark Results                       ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

printf "%-20s %8s %8s %8s %8s\n" "Test Name" "Avg(ms)" "Min(ms)" "Max(ms)" "Count"
printf "%-20s %8s %8s %8s %8s\n" "--------------------" "--------" "--------" "--------" "--------"

# Build JSON and print table
JSON_RESULTS=""
FIRST=true

for test in simple_symbol wildcard_search with_body code_only exclude_pattern concept_search batch_search multi_language; do
    sum=$(cat "$TEMP_DIR/${test}_sum")
    min=$(cat "$TEMP_DIR/${test}_min")
    max=$(cat "$TEMP_DIR/${test}_max")
    count=$(cat "$TEMP_DIR/${test}_count")

    if ((count > 0)); then
        avg=$((sum / count))

        # Fix min if no valid results
        if ((min == 999999)); then
            min=0
        fi

        printf "%-20s %8d %8d %8d %8d\n" "$test" "$avg" "$min" "$max" "$count"

        if [[ "$FIRST" == "true" ]]; then
            FIRST=false
        else
            JSON_RESULTS+=","
        fi
        JSON_RESULTS+="\"$test\":{\"avg_ms\":$avg,\"min_ms\":$min,\"max_ms\":$max,\"count\":$count}"
    fi
done

echo ""
echo "Total benchmark time: ${TOTAL_TIME}s"
if ((TOTAL_TIME > 0)); then
    echo "Tests per second: $((ITERATIONS * 8 / TOTAL_TIME))"
fi
echo ""

# Generate JSON output
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
cat > "$OUTPUT_FILE" << EOF
{
  "benchmark": {
    "iterations": $ITERATIONS,
    "total_time_seconds": $TOTAL_TIME,
    "timestamp": "$TIMESTAMP",
    "results": {
      $JSON_RESULTS
    }
  }
}
EOF

echo "Results saved to: $OUTPUT_FILE"
echo ""
echo "JSON Output:"
cat "$OUTPUT_FILE"
