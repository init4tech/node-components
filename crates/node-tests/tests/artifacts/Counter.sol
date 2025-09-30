// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Counter {
    uint256 public count;
    event Count(uint256 indexed count);

    constructor() {
        count = 0;
    }
    function increment() public {
        count += 1;
        emit Count(count);
    }
}