#!/bin/bash

# Script pour exécuter Clippy sur tous les crates du workspace
# et collecter tous les résultats sans s'arrêter au premier échec

set -e

# Couleurs pour l'affichage
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Liste des crates du workspace
CRATES=(
    "ractor"
    "ractor_cluster"
    "ractor_cluster_derive"
    "ractor_playground"
    "ractor_cluster_integration_tests"
    "ractor_example_entry_proc"
    "xtask"
)

# Variables pour collecter les résultats
PASSED_CRATES=()
FAILED_CRATES=()
ALL_ERRORS=""

echo -e "${BLUE}=== EXÉCUTION DE CLIPPY SUR TOUS LES CRATES ===${NC}"
echo ""

# Boucle sur tous les crates
for crate in "${CRATES[@]}"; do
    echo -e "${YELLOW}>>> Testing ${crate} <<<${NC}"

    # Exécuter clippy et capturer la sortie
    if output=$(cargo clippy --package "$crate" --all-targets -- -D clippy::all -D warnings 2>&1); then
        echo -e "${GREEN}✅ ${crate}: PASS${NC}"
        PASSED_CRATES+=("$crate")
    else
        echo -e "${RED}❌ ${crate}: FAIL${NC}"
        FAILED_CRATES+=("$crate")
        ALL_ERRORS+="=== ERREURS POUR ${crate} ===\n"
        ALL_ERRORS+="$output\n\n"
    fi
    echo ""
done

# Résumé final
echo -e "${BLUE}=== RÉSUMÉ FINAL ===${NC}"
echo ""

if [ ${#PASSED_CRATES[@]} -gt 0 ]; then
    echo -e "${GREEN}Crates qui passent Clippy (${#PASSED_CRATES[@]}):"
    for crate in "${PASSED_CRATES[@]}"; do
        echo -e "  ✅ $crate"
    done
    echo ""
fi

if [ ${#FAILED_CRATES[@]} -gt 0 ]; then
    echo -e "${RED}Crates qui échouent à Clippy (${#FAILED_CRATES[@]}):"
    for crate in "${FAILED_CRATES[@]}"; do
        echo -e "  ❌ $crate"
    done
    echo ""

    echo -e "${RED}=== DÉTAILS DES ERREURS ===${NC}"
    echo -e "$ALL_ERRORS"
fi

# Statistiques
total_crates=${#CRATES[@]}
passed_count=${#PASSED_CRATES[@]}
failed_count=${#FAILED_CRATES[@]}

echo -e "${BLUE}Statistiques: ${passed_count}/${total_crates} crates passent Clippy${NC}"

# Exit avec le bon code de retour
if [ ${#FAILED_CRATES[@]} -gt 0 ]; then
    echo -e "${RED}Des erreurs Clippy ont été trouvées dans ${failed_count} crate(s).${NC}"
    exit 1
else
    echo -e "${GREEN}Tous les crates passent Clippy avec succès !${NC}"
    exit 0
fi
