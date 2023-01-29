#!/bin/bash
# Finds expected version of a crate in another crate's Cargo.toml file
get_crate_expected_version_number()
{
    local INDEX_CRATE=$1
    local TARGET_CRATE=$2

    local INDEX_TOML_FILE="$INDEX_CRATE/Cargo.toml"
    # Issue #174. The script is looking for multiple entries if the dependency is listed multiple times
    # Additionally this regex with grep works for both the notations
    # 1. crate = { some_other_options ... version = "x.y.z" ... other_options }
    # 2. crate = "x.y.z"
    local EXPECTED_VERSION=$(grep "$TARGET_CRATE" $INDEX_TOML_FILE | grep -o '[0-9]\.[0-9\.]\+' | head -n 1)
    echo $EXPECTED_VERSION
}

# Get published versions of a crate from https://github.com/rust-lang/crates.io-index/
get_crate_published_versions()
{
    local CRATE_INDEX_URL=$1

    local PUBLISHED_VERSIONS=$(curl -sS "$CRATE_INDEX_URL" | jq .vers)
    echo "$PUBLISHED_VERSIONS"
}

# Retrieve the raw github url for a given crate based on the crate name following
# crates.io's strange indexing strategy
get_crate_raw_github_url() {
    local CRATE=$1

    local STR_LEN=$(echo "$CRATE" | wc -c)
    STR_LEN=$((STR_LEN - 1))
    if (($STR_LEN > 3)); then
        local FIRST_TWO=$(echo ${CRATE:0:2})
        local SECOND_TWO=$(echo ${CRATE:2:2})
        echo "https://raw.githubusercontent.com/rust-lang/crates.io-index/master/$FIRST_TWO/$SECOND_TWO/$CRATE"
    else
        local FIRST_ONE=$(echo ${CRATE:0:1})
        echo "https://raw.githubusercontent.com/rust-lang/crates.io-index/master/$STR_LEN/$FIRST_ONE/$CRATE"
    fi
}

# Wait for a specific crate version to be published to crates.io.
# See https://github.com/novifinancial/akd/issues/116.
# Must be run in the project root folder.
INDEX_CRATE=$1
TARGET_CRATE=$2

if [ "$INDEX_CRATE" == "" ] || [ "$TARGET_CRATE" == "" ]
then
    echo "Both the target crate and index crate are required arguments."
    echo "Usage:"
    echo "bash ./.github/workflows/wait-for-crate-dependency.sh INDEX_CRATE TARGET_CRATE"
    echo " - INDEX_CRATE  : The crate which contains the dependency version specification"
    echo " - TARGET_CRATE : The crate which version needs to be published to build the INDEX_CRATE"
    exit 1
fi

EXPECTED_VERSION=$(get_crate_expected_version_number "$INDEX_CRATE" "$TARGET_CRATE" || exit 1)
echo "Expecting $TARGET_CRATE = { version = $EXPECTED_VERSION } for $INDEX_CRATE"
TARGET_URL=$(get_crate_raw_github_url "$TARGET_CRATE" || exit 1)
echo "Target URL for $TARGET_CRATE is $TARGET_URL"
WAIT_TIME=1
while sleep $WAIT_TIME;
do
    PUBLISHED_VERSIONS=$(get_crate_published_versions "$TARGET_URL" | tr '\n' " ")
    echo "Available $TARGET_CRATE versions: $PUBLISHED_VERSIONS"
    EXISTS=$(echo $PUBLISHED_VERSIONS | grep "\"$EXPECTED_VERSION\"")
    if [[ $EXISTS != "" ]]; then
        echo "Expected version of $TARGET_CRATE ($EXPECTED_VERSION) has been published"
        break
    fi
    echo "Expected version of $TARGET_CRATE ($EXPECTED_VERSION) is not yet published. Retrying after a wait"
    WAIT_TIME=$((WAIT_TIME+1))
    if [[ $WAIT_TIME == 42 ]]; then
        echo "Giving up after 42 wait periods"
        exit 1
    fi
done
