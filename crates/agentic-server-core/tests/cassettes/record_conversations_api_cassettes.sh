#!/usr/bin/env bash
# Record Conversations API CRUD operations against OpenAI and gateway.
#
# Usage:
#   OPENAI_API_KEY=sk-... bash record_conversations_api_cassettes.sh
#   CONVERSATIONS_RECORD_SET=openai OPENAI_API_KEY=sk-... bash record_conversations_api_cassettes.sh
#   CONVERSATIONS_RECORD_SET=gateway GATEWAY_URL=http://localhost:9000 bash record_conversations_api_cassettes.sh
#
# Environment:
#   CONVERSATIONS_RECORD_SET    "all" (default), "openai", or "gateway"
#   OPENAI_API_KEY              OpenAI API key (required for OpenAI recordings)
#   OPENAI_MODEL                OpenAI model (default: gpt-4o)
#   GATEWAY_URL                 Gateway URL (default: http://localhost:9000)
#   GATEWAY_MODEL               Gateway model (default: gpt-4o)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CASSETTES_DIR="$SCRIPT_DIR/conversations"
RECORD_SET="${CONVERSATIONS_RECORD_SET:-all}"
OPENAI_MODEL="${OPENAI_MODEL:-gpt-4o}"
GATEWAY_URL="${GATEWAY_URL:-http://localhost:9000}"
GATEWAY_MODEL="${GATEWAY_MODEL:-gpt-4o}"

mkdir -p "$CASSETTES_DIR"

# Validator: checks that recorded cassettes match expected structure
validate_cassette() {
    local cassette_file="$1"
    local scenario="$2"

    echo "  Validating $scenario..."

    if ! python3 -c "
import sys
from pathlib import Path
from yaml import safe_load

cassette = safe_load(Path('$cassette_file').read_text())
turns = cassette.get('turns', [])

if not turns:
    print(f'ERROR: No turns recorded in $cassette_file', file=sys.stderr)
    sys.exit(1)

# Check that conversation operations are recorded
conversation_ops = {
    'create': False,
    'retrieve': False,
    'update': False,
    'delete': False,
}

item_ops = {
    'create': False,
    'list': False,
    'retrieve': False,
    'delete': False,
}

for turn in turns:
    req = turn.get('request', {})
    path = req.get('path', '')
    method = req.get('method', '')

    # Track conversation operations
    if '/v1/conversations' in path and not '/items' in path:
        if method == 'POST' and path == '/v1/conversations':
            conversation_ops['create'] = True
        elif method == 'GET' and path.count('/') == 3:
            conversation_ops['retrieve'] = True
        elif method == 'PATCH':
            conversation_ops['update'] = True
        elif method == 'DELETE' and path.count('/') == 3:
            conversation_ops['delete'] = True

    # Track item operations
    if '/items' in path:
        if method == 'POST':
            item_ops['create'] = True
        elif method == 'GET' and '?' in path:
            item_ops['list'] = True
        elif method == 'GET' and path.count('/') == 5:
            item_ops['retrieve'] = True
        elif method == 'DELETE':
            item_ops['delete'] = True

# Scenario-specific validation
scenario = '$scenario'
if 'crud' in scenario:
    missing_conv = [op for op, found in conversation_ops.items() if not found]
    missing_item = [op for op, found in item_ops.items() if not found]

    if missing_conv:
        print(f'ERROR: Missing conversation operations: {missing_conv}', file=sys.stderr)
        sys.exit(1)

    if missing_item:
        print(f'ERROR: Missing item operations: {missing_item}', file=sys.stderr)
        sys.exit(1)

print(f'✓ Validated $scenario')
"; then
        echo "ERROR: Validation failed for $cassette_file"
        return 1
    fi

    return 0
}

# Record a single scenario
record_scenario() {
    local provider="$1"
    local scenario="$2"
    local output_file="$3"
    shift 3
    local extra_args=("$@")

    local temp_file="${output_file}.tmp"

    echo "Recording $provider/$scenario..."

    if [[ "$provider" == "openai" ]]; then
        if [[ -z "${OPENAI_API_KEY:-}" ]]; then
            echo "ERROR: OPENAI_API_KEY required for OpenAI recordings"
            return 1
        fi

        python3 "$SCRIPT_DIR/record_conversations_crud.py" \
            --provider openai \
            --model "$OPENAI_MODEL" \
            --scenario "$scenario" \
            --output "$temp_file" \
            "${extra_args[@]}"
    else
        python3 "$SCRIPT_DIR/record_conversations_crud.py" \
            --provider gateway \
            --gateway-url "$GATEWAY_URL" \
            --model "$GATEWAY_MODEL" \
            --scenario "$scenario" \
            --output "$temp_file" \
            "${extra_args[@]}"
    fi

    # Validate before committing
    if validate_cassette "$temp_file" "$scenario"; then
        mv "$temp_file" "$output_file"
        echo "✓ Recorded $provider/$scenario -> $output_file"
    else
        echo "✗ Validation failed, keeping temp file: $temp_file"
        return 1
    fi
}

# Scenarios to record
scenarios=(
    "basic-crud:Basic conversation and item CRUD operations"
    "pagination:Item pagination with order and cursor"
    "stateful-context:Manually added items affecting model context"
    "deletion-continuation:Item deletion affecting continuation"
    "branching:Branching before/after manual item additions"
    "error-cases:Invalid requests and error responses"
)

should_record_openai() {
    [[ "$RECORD_SET" == "all" || "$RECORD_SET" == "openai" ]]
}

should_record_gateway() {
    [[ "$RECORD_SET" == "all" || "$RECORD_SET" == "gateway" ]]
}

echo "=========================================="
echo "Conversations API Cassette Recorder"
echo "=========================================="
echo "Record set: $RECORD_SET"
echo "OpenAI model: $OPENAI_MODEL"
echo "Gateway URL: $GATEWAY_URL"
echo "Gateway model: $GATEWAY_MODEL"
echo ""

failed=()

for scenario_spec in "${scenarios[@]}"; do
    IFS=':' read -r scenario description <<< "$scenario_spec"

    echo ""
    echo "📹 $description"
    echo "----------------------------------------"

    if should_record_openai; then
        if ! record_scenario "openai" "$scenario" \
            "$CASSETTES_DIR/conversations-${scenario}-openai.yaml"; then
            failed+=("openai/$scenario")
        fi
    fi

    if should_record_gateway; then
        if ! record_scenario "gateway" "$scenario" \
            "$CASSETTES_DIR/conversations-${scenario}-gateway.yaml"; then
            failed+=("gateway/$scenario")
        fi
    fi
done

echo ""
echo "=========================================="
if [[ ${#failed[@]} -eq 0 ]]; then
    echo "✓ All scenarios recorded successfully"
    exit 0
else
    echo "✗ Failed scenarios:"
    for f in "${failed[@]}"; do
        echo "  - $f"
    done
    exit 1
fi
