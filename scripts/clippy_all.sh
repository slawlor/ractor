#!/bin/bash

# Script to run Clippy on all workspace crates
# and collect all results without stopping at first failure

set -e

# Colors for display
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# List of workspace crates
CRATES=(
    "ractor"
    "ractor_cluster"
    "ractor_cluster_derive"
    "ractor_playground"
    "ractor_cluster_integration_tests"
    "ractor_example_entry_proc"
    "xtask"
)

# Variables to collect results
PASSED_CRATES=()
FAILED_CRATES=()
ALL_ERRORS=""

echo -e "${BLUE}=== RUNNING CLIPPY ON ALL CRATES ===${NC}"
echo ""

# Loop through all crates
for crate in "${CRATES[@]}"; do
    echo -e "${YELLOW}>>> Testing ${crate} <<<${NC}"

    # Run clippy and capture output
    if output=$(cargo clippy --package "$crate" --all-targets -- -D clippy::all -D warnings 2>&1); then
        echo -e "${GREEN}✅ ${crate}: PASS${NC}"
        PASSED_CRATES+=("$crate")
    else
        echo -e "${RED}❌ ${crate}: FAIL${NC}"
        FAILED_CRATES+=("$crate")
        ALL_ERRORS+="=== ERRORS FOR ${crate} ===\n"
        ALL_ERRORS+="$output\n\n"
    fi
    echo ""
done

# Final summary
echo -e "${BLUE}=== FINAL SUMMARY ===${NC}"
echo ""

if [ ${#PASSED_CRATES[@]} -gt 0 ]; then
    echo -e "${GREEN}Crates passing Clippy (${#PASSED_CRATES[@]}):"
    for crate in "${PASSED_CRATES[@]}"; do
        echo -e "  ✅ $crate"
    done
    echo ""
fi

if [ ${#FAILED_CRATES[@]} -gt 0 ]; then
    echo -e "${RED}Crates failing Clippy (${#FAILED_CRATES[@]}):"
    for crate in "${FAILED_CRATES[@]}"; do
        echo -e "  ❌ $crate"
    done
    echo ""

    echo -e "${RED}=== ERROR DETAILS ===${NC}"
    echo -e "$ALL_ERRORS"
fi

# Statistics
total_crates=${#CRATES[@]}
passed_count=${#PASSED_CRATES[@]}
failed_count=${#FAILED_CRATES[@]}

echo -e "${BLUE}Statistics: ${passed_count}/${total_crates} crates pass Clippy${NC}"

# Exit with appropriate return code
if [ ${#FAILED_CRATES[@]} -gt 0 ]; then
    echo -e "${RED}Clippy errors found in ${failed_count} crate(s).${NC}"
    exit 1
else
    echo -e "${GREEN}All crates pass Clippy successfully!${NC}"
    exit 0
fi
