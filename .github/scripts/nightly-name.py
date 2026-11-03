#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""A nightly's name from its number: a colour and a flower bees visit
("Amber Clover"). Releases are named for a bee's life (0.1 Pollen); nightlies
for what's in bloom. 23 colours and 25 flowers, so 575 nightlies in a row
never share a name."""

import sys

COLOURS = [
    "Amber", "Golden", "Honey", "Saffron", "Copper", "Coral", "Rose", "Ruby",
    "Plum", "Violet", "Indigo", "Azure", "Teal", "Jade", "Lilac", "Olive",
    "Ivory", "Pearl", "Silver", "Dusk", "Dawn", "Velvet", "Ember",
]
FLOWERS = [
    "Clover", "Lavender", "Heather", "Thyme", "Borage", "Sunflower", "Poppy",
    "Linden", "Acacia", "Sage", "Rosemary", "Bluebell", "Crocus", "Marigold",
    "Foxglove", "Cosmos", "Aster", "Mallow", "Phacelia", "Sedum", "Catmint",
    "Cornflower", "Willow", "Hawthorn", "Bramble",
]


def name(number: int) -> str:
    return f"{COLOURS[number % len(COLOURS)]} {FLOWERS[number % len(FLOWERS)]}"


if __name__ == "__main__":
    print(name(int(sys.argv[1])))
