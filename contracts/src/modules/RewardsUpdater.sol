// SPDX-License-Identifier: BUSL-1.1
pragma solidity ^0.8.24;

import {PoolId} from "v4-core/src/types/PoolId.sol";
import {PoolRewards, REWARD_GROWTH_SIZE} from "../types/PoolRewards.sol";
import {CalldataReader} from "../types/CalldataReader.sol";

import {TickLib} from "../libraries/TickLib.sol";
import {MixedSignLib} from "../libraries/MixedSignLib.sol";
import {FixedPointMathLib} from "solady/src/utils/FixedPointMathLib.sol";

import {console} from "forge-std/console.sol";
import {DEBUG_LOGS} from "./DevFlags.sol";
import {FormatLib} from "super-sol/libraries/FormatLib.sol";

/// @author philogy <https://github.com/philogy>
abstract contract RewardsUpdater {
    using FixedPointMathLib for uint256;
    using TickLib for uint256;
    using FormatLib for *;

    error WrongEndLiquidity(uint128 endLiquidity, uint128 actualCurrentLiquidity);

    function _decodeAndReward(CalldataReader reader, PoolRewards storage poolRewards, PoolId id)
        internal
        returns (CalldataReader, uint256 total)
    {
        if (DEBUG_LOGS) {
            console.log(
                "[RewardsUpdater] Entering _decodeAndReward(reader, poolRewards, %x)", uint256(PoolId.unwrap(id))
            );
        }
        uint256 cumulativeGrowth;
        uint128 endLiquidity;
        {
            if (DEBUG_LOGS) console.log("[RewardsUpdater] Decoding below flag");
            bool below;
            {
                uint8 rewardUpdateVariantMap;
                (reader, rewardUpdateVariantMap) = reader.readU8();
                below = rewardUpdateVariantMap & 1 != 0;
            }
            if (DEBUG_LOGS) console.log("[RewardsUpdater] Decoding startTick");
            int24 startTick;
            (reader, startTick) = reader.readI24();
            if (DEBUG_LOGS) console.log("[RewardsUpdater] Retrieving pool %x current tick", uint256(PoolId.unwrap(id)));
            int24 currentTick = _getCurrentTick(id);
            if (DEBUG_LOGS) console.log("[RewardsUpdater] Decoding update start liquidity");
            uint128 liquidity;
            (reader, liquidity) = reader.readU128();
            if (DEBUG_LOGS) console.log("[RewardsUpdater] Decoding bounds of reward amounts list");
            CalldataReader amountsEnd;
            (reader, amountsEnd) = reader.readU24End();
            if (DEBUG_LOGS) console.log("[RewardsUpdater] Starting core reward loop");
            (reader, total, cumulativeGrowth, endLiquidity) = below
                ? _rewardBelow(poolRewards.rewardGrowthOutside, currentTick, reader, startTick, id, liquidity, amountsEnd)
                : _rewardAbove(poolRewards.rewardGrowthOutside, currentTick, reader, startTick, id, liquidity, amountsEnd);
        }

        if (DEBUG_LOGS) console.log("[RewardsUpdater] Completed core reward loop, checking end liquidity");

        uint128 currentLiquidity = _getCurrentLiquidity(id);
        if (DEBUG_LOGS) {
            console.log("[RewardsUpdater] endLiquidity: %s", endLiquidity.fmtD(18));
            console.log("[RewardsUpdater] currentLiquidity: %s", currentLiquidity.fmtD(18));
        }
        if (endLiquidity != currentLiquidity) revert WrongEndLiquidity(endLiquidity, currentLiquidity);

        if (DEBUG_LOGS) console.log("[RewardsUpdater] Updating global growth by cumulativeGrowth");

        poolRewards.globalGrowth += cumulativeGrowth;

        return (reader, total);
    }

    function _rewardBelow(
        uint256[REWARD_GROWTH_SIZE] storage rewardGrowthOutside,
        int24 currentTick,
        CalldataReader reader,
        int24 rewardTick,
        PoolId id,
        uint128 liquidity,
        CalldataReader amountsEnd
    ) internal returns (CalldataReader, uint256, uint256, uint128) {
        if (DEBUG_LOGS) console.log("[RewardsUpdater] entering _rewardBelow");

        bool initialized = true;
        uint256 total = 0;
        uint256 cumulativeGrowth = 0;

        if (DEBUG_LOGS) console.log("[RewardsUpdater] total: %s", total.fmtD(18));

        while (true) {
            if (DEBUG_LOGS) {
                console.log(
                    "[RewardsUpdater] reward update loop (initialized: %s, tick: %s, liquidity: %s)",
                    initialized.toStr(),
                    rewardTick.toStr(),
                    liquidity.fmtD(18)
                );
            }
            if (initialized) {
                if (DEBUG_LOGS) console.log("[RewardsUpdater] Initialized, updating tick");
                // Amounts beyond the end of the sequence default to 0.
                uint256 amount;
                (reader, amount) = reader.readU128();
                total += amount;

                cumulativeGrowth += flatDivWad(amount, liquidity);

                // Break *before* we update the cumulative growth and net liquidity of ticks as we
                // don't want to be doing that for the reward to the current tick.
                if (rewardTick > currentTick) break;

                rewardGrowthOutside[uint24(rewardTick)] += cumulativeGrowth;

                {
                    int128 netLiquidity = _getNetTickLiquidity(id, rewardTick);
                    if (DEBUG_LOGS) console.log("[RewardsUpdater] Net liquidity: %s", netLiquidity.fmtD(18));
                    liquidity = MixedSignLib.add(liquidity, netLiquidity);
                }

                if (DEBUG_LOGS) {
                    console.log("[RewardsUpdater] Adding %s to tick %s", amount.fmtD(18), rewardTick.toStr());
                    console.log("[RewardsUpdater] New total: %s", total.fmtD(18));
                    console.log(
                        "[RewardsUpdater] Increasing tick %s growth outside by %s",
                        rewardTick.toStr(),
                        cumulativeGrowth.fmtD(18)
                    );
                    console.log("[RewardsUpdater] Retrieved and updated liquidity to: %s", liquidity.fmtD(18));
                }
            } else if (rewardTick > currentTick) {
                break;
            }

            (initialized, rewardTick) = _findNextTickUp(id, rewardTick);
        }

        reader.requireAtEndOf(amountsEnd);

        if (DEBUG_LOGS) {
            console.log("[RewardsUpdater] Main reward loop complete.");
            console.log(
                "[RewardsUpdater] Final values (total: %s, cumulativeGrowth: %s)",
                total.fmtD(18),
                cumulativeGrowth.fmtD(18)
            );
        }

        return (reader, total, cumulativeGrowth, liquidity);
    }

    function _rewardAbove(
        uint256[REWARD_GROWTH_SIZE] storage rewardGrowthOutside,
        int24 currentTick,
        CalldataReader reader,
        int24 rewardTick,
        PoolId id,
        uint128 liquidity,
        CalldataReader amountsEnd
    ) internal returns (CalldataReader, uint256, uint256, uint128) {
        if (DEBUG_LOGS) console.log("[RewardsUpdater] entering _rewardAbove");

        bool initialized = true;
        uint256 total = 0;
        uint256 cumulativeGrowth = 0;

        while (true) {
            if (initialized) {
                if (DEBUG_LOGS) console.log("[RewardsUpdater] Initialized, updating tick");
                // Amounts beyond the end of the sequence default to 0.
                uint256 amount = 0;
                if (reader != amountsEnd) (reader, amount) = reader.readU128();
                total += amount;

                cumulativeGrowth += flatDivWad(amount, liquidity);

                if (rewardTick <= currentTick) break;

                rewardGrowthOutside[uint24(rewardTick)] += cumulativeGrowth;

                liquidity = MixedSignLib.sub(liquidity, _getNetTickLiquidity(id, rewardTick));

                if (DEBUG_LOGS) {
                    console.log("[RewardsUpdater] Adding %s to tick %s", amount.fmtD(18), rewardTick.toStr());
                    console.log("[RewardsUpdater] New total: %s", total.fmtD(18));
                    console.log(
                        "[RewardsUpdater] Increasing tick %s growth outside by %s",
                        rewardTick.toStr(),
                        cumulativeGrowth.fmtD(18)
                    );
                    console.log("[RewardsUpdater] Retrieved and updated liquidity to: %s", liquidity.fmtD(18));
                }
            } else if (rewardTick <= currentTick) {
                break;
            }

            (initialized, rewardTick) = _findNextTickDown(id, rewardTick);
        }

        if (DEBUG_LOGS) {
            console.log("[RewardsUpdater] Main reward loop complete.");
            console.log(
                "[RewardsUpdater] Final values (total: %s, cumulativeGrowth: %s)",
                total.fmtD(18),
                cumulativeGrowth.fmtD(18)
            );
        }

        reader.requireAtEndOf(amountsEnd);

        return (reader, total, cumulativeGrowth, liquidity);
    }

    function _findNextTickUp(PoolId id, int24 tick) internal view returns (bool initialized, int24 newTick) {
        (int16 wordPos, uint8 bitPos) = TickLib.position(TickLib.compress(tick) + 1);
        (initialized, bitPos) = _getPoolBitmapInfo(id, wordPos).nextBitPosGte(bitPos);
        newTick = TickLib.toTick(wordPos, bitPos);
    }

    function _findNextTickDown(PoolId id, int24 tick) internal view returns (bool initialized, int24 newTick) {
        (int16 wordPos, uint8 bitPos) = TickLib.position(TickLib.compress(tick - 1));
        (initialized, bitPos) = _getPoolBitmapInfo(id, wordPos).nextBitPosLte(bitPos);
        newTick = TickLib.toTick(wordPos, bitPos);
    }

    /**
     * @dev Overflow-safe fixed-point division of `x / y` resulting in `0` if `y` is zero.
     */
    function flatDivWad(uint256 x, uint256 y) internal pure returns (uint256) {
        return (x * FixedPointMathLib.WAD).rawDiv(y);
    }

    ////////////////////////////////////////////////////////////////
    //                       POOL INTERFACE                       //
    ////////////////////////////////////////////////////////////////

    function _getPoolBitmapInfo(PoolId id, int16 wordPos) internal view virtual returns (uint256);
    function _getNetTickLiquidity(PoolId id, int24 tick) internal view virtual returns (int128);
    function _getCurrentLiquidity(PoolId id) internal view virtual returns (uint128);
    function _getCurrentTick(PoolId id) internal view virtual returns (int24);
}