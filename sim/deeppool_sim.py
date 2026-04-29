"""
DeepPool Economic Simulator

Pure-Python simulation of the DeepPool constant-product AMM.
Tests: fee compounding, k growth, LP value appreciation, manipulation resistance.
No external deps — stdlib only.
"""

from __future__ import annotations
import random
import math

# ============================================================================
# Constants
# ============================================================================

SWAP_FEE_BPS = 25
FEE_DENOMINATOR = 10000
MIN_LIQUIDITY = 1000
LAMPORTS_PER_SOL = 1_000_000_000
TOKEN_DECIMALS = 6
TOKEN_MULT = 10 ** TOKEN_DECIMALS

# ============================================================================
# Pool
# ============================================================================

class DeepPool:
    def __init__(self, initial_sol: int, initial_tokens: int):
        assert initial_sol > 0 and initial_tokens > 0
        self.sol_reserve = initial_sol
        self.token_reserve = initial_tokens
        self.lp_supply = int(math.isqrt(initial_sol * initial_tokens)) - MIN_LIQUIDITY
        assert self.lp_supply > 0
        self.total_swaps = 0
        self.total_fees_sol = 0
        self.total_fees_tokens = 0

    @property
    def k(self) -> int:
        return self.sol_reserve * self.token_reserve

    @property
    def price(self) -> float:
        if self.token_reserve == 0:
            return float('inf')
        return self.sol_reserve / self.token_reserve

    @property
    def lp_value_sol(self) -> float:
        """Value of 1 LP token in SOL (both sides valued at pool price)."""
        if self.lp_supply == 0:
            return 0.0
        sol_per_lp = self.sol_reserve / self.lp_supply
        tokens_per_lp = self.token_reserve / self.lp_supply
        return sol_per_lp + tokens_per_lp * self.price

    def swap_buy(self, sol_in: int) -> int:
        """SOL → Token. Returns tokens out."""
        assert sol_in > 0
        fee = (sol_in * SWAP_FEE_BPS) // FEE_DENOMINATOR
        effective = sol_in - fee
        tokens_out = (effective * self.token_reserve) // (self.sol_reserve + effective)
        tokens_out = min(tokens_out, self.token_reserve - 1)

        self.sol_reserve += sol_in  # full amount (fee stays)
        self.token_reserve -= tokens_out
        self.total_swaps += 1
        self.total_fees_sol += fee
        return tokens_out

    def swap_sell(self, tokens_in: int, transfer_fee_bps: int = 0) -> int:
        """Token → SOL. Returns SOL out."""
        assert tokens_in > 0
        # Token-2022 transfer fee on input
        transfer_fee = (tokens_in * transfer_fee_bps) // FEE_DENOMINATOR
        net_in = tokens_in - transfer_fee

        fee = (net_in * SWAP_FEE_BPS) // FEE_DENOMINATOR
        effective = net_in - fee
        sol_out = (effective * self.sol_reserve) // (self.token_reserve + effective)
        sol_out = min(sol_out, self.sol_reserve - 1)

        self.token_reserve += net_in  # net amount after transfer fee (swap fee stays)
        self.sol_reserve -= sol_out
        self.total_swaps += 1
        self.total_fees_tokens += fee
        return sol_out

    def add_liquidity(self, token_amount: int, min_lp_out: int = 0) -> tuple[int, int]:
        """Add proportional liquidity. Returns (sol_required, lp_minted)."""
        sol_required = (token_amount * self.sol_reserve) // self.token_reserve
        lp_minted = (self.lp_supply * token_amount) // self.token_reserve

        assert lp_minted >= min_lp_out, f"LP output {lp_minted} below minimum {min_lp_out}"

        self.sol_reserve += sol_required
        self.token_reserve += token_amount
        self.lp_supply += lp_minted
        return sol_required, lp_minted

    def remove_liquidity(self, lp_amount: int) -> tuple[int, int]:
        """Remove proportional liquidity. Returns (sol_out, tokens_out)."""
        sol_out = (lp_amount * self.sol_reserve) // self.lp_supply
        tokens_out = (lp_amount * self.token_reserve) // self.lp_supply

        self.sol_reserve -= sol_out
        self.token_reserve -= tokens_out
        self.lp_supply -= lp_amount
        return sol_out, tokens_out

    def print_state(self, label: str = ""):
        print(f"\n{'='*50}")
        if label:
            print(f"  {label}")
        print(f"{'='*50}")
        print(f"  SOL:    {self.sol_reserve / LAMPORTS_PER_SOL:.4f}")
        print(f"  Tokens: {self.token_reserve / TOKEN_MULT:,.0f}")
        print(f"  Price:  {self.price * TOKEN_MULT:.6f} SOL/token")
        print(f"  K:      {self.k:,}")
        print(f"  LP supply: {self.lp_supply:,}")
        total_lp_value = self.lp_value_sol * self.lp_supply
        print(f"  LP value:  {total_lp_value / LAMPORTS_PER_SOL:.4f} SOL total ({self.lp_value_sol * TOKEN_MULT:.10f} SOL/LP)")
        print(f"  Swaps:  {self.total_swaps}")


# ============================================================================
# Scenarios
# ============================================================================

def scenario_fee_compounding(seed=42):
    """Show K growing over many swaps."""
    print("\n" + "="*50)
    print("  SCENARIO: Fee Compounding")
    print("="*50)

    rng = random.Random(seed)
    pool = DeepPool(200 * LAMPORTS_PER_SOL, 150_000_000 * TOKEN_MULT)
    k_initial = pool.k
    lp_value_initial = pool.lp_value_sol

    pool.print_state("Initial")

    # 1000 random swaps
    for i in range(1000):
        if rng.random() < 0.5:
            sol = rng.randint(1, 10) * LAMPORTS_PER_SOL // 10
            pool.swap_buy(sol)
        else:
            tokens = rng.randint(1, 1000) * 1000 * TOKEN_MULT
            pool.swap_sell(tokens)

    pool.print_state("After 1000 swaps")

    k_final = pool.k
    k_growth = (k_final - k_initial) / k_initial * 100
    lp_growth = (pool.lp_value_sol - lp_value_initial) / lp_value_initial * 100

    print(f"\n  K growth: {k_growth:.4f}%")
    print(f"  LP value growth: {lp_growth:.4f}%")
    print(f"  Total fees (SOL side): {pool.total_fees_sol / LAMPORTS_PER_SOL:.4f} SOL")
    print(f"  Total fees (token side): {pool.total_fees_tokens / TOKEN_MULT:,.0f} tokens")
    assert k_final > k_initial, "K must increase"
    print("  ✓ K increased")


def scenario_manipulation_cost(seed=123):
    """Show cost of manipulating price."""
    print("\n" + "="*50)
    print("  SCENARIO: Manipulation Cost")
    print("="*50)

    pool = DeepPool(200 * LAMPORTS_PER_SOL, 150_000_000 * TOKEN_MULT)
    price_before = pool.price

    # Try to move price 20% with a single trade
    # Need: delta_x = x * (sqrt(1.2) - 1) ≈ 0.095 * 200 = 19 SOL
    attack_sol = 19 * LAMPORTS_PER_SOL
    tokens_received = pool.swap_buy(attack_sol)
    price_after = pool.price

    price_change = (price_after - price_before) / price_before * 100
    cost_sol = attack_sol / LAMPORTS_PER_SOL

    print(f"  Attack: {cost_sol} SOL buy")
    print(f"  Price change: {price_change:.2f}%")
    print(f"  Tokens received: {tokens_received / TOKEN_MULT:,.0f}")

    # Now sell those tokens back
    sol_back = pool.swap_sell(tokens_received)
    net_loss = attack_sol - sol_back

    print(f"  SOL back from sell: {sol_back / LAMPORTS_PER_SOL:.4f}")
    print(f"  Net loss: {net_loss / LAMPORTS_PER_SOL:.4f} SOL")
    print(f"  Attack cost: {net_loss / LAMPORTS_PER_SOL:.4f} SOL ({net_loss * 100 / attack_sol:.2f}%)")
    assert net_loss > 0, "Manipulation must cost money"
    print("  ✓ Manipulation is unprofitable")


def scenario_lp_appreciation(seed=456):
    """Show LP value growing from fees."""
    print("\n" + "="*50)
    print("  SCENARIO: LP Value Appreciation")
    print("="*50)

    rng = random.Random(seed)
    pool = DeepPool(200 * LAMPORTS_PER_SOL, 150_000_000 * TOKEN_MULT)

    # LP holder adds 10% more liquidity
    sol_added, lp_minted = pool.add_liquidity(15_000_000 * TOKEN_MULT)
    initial_lp = lp_minted
    initial_value = (initial_lp * pool.sol_reserve) // pool.lp_supply

    print(f"  LP holder added: {sol_added / LAMPORTS_PER_SOL:.4f} SOL + 15M tokens")
    print(f"  LP received: {lp_minted:,}")
    print(f"  Initial LP value: {initial_value / LAMPORTS_PER_SOL:.4f} SOL")

    # 5000 swaps of trading
    for _ in range(5000):
        if rng.random() < 0.5:
            sol = rng.randint(1, 20) * LAMPORTS_PER_SOL // 10
            pool.swap_buy(sol)
        else:
            tokens = rng.randint(1, 2000) * 1000 * TOKEN_MULT
            pool.swap_sell(tokens)

    # LP holder redeems
    final_value_sol = (initial_lp * pool.sol_reserve) // pool.lp_supply
    final_value_tokens = (initial_lp * pool.token_reserve) // pool.lp_supply
    value_growth = (final_value_sol - initial_value) / initial_value * 100

    # Total value = SOL side + token side valued at current price
    total_value = final_value_sol + int(final_value_tokens * pool.price)
    initial_total = initial_value + int((initial_lp * pool.token_reserve) // pool.lp_supply * (pool.sol_reserve / pool.token_reserve))

    print(f"\n  After 5000 swaps:")
    print(f"  LP SOL side: {final_value_sol / LAMPORTS_PER_SOL:.4f} SOL")
    print(f"  LP token side: {final_value_tokens / TOKEN_MULT:,.0f} tokens")
    print(f"  K growth: {(pool.k - (200 * LAMPORTS_PER_SOL * 150_000_000 * TOKEN_MULT)) / (200 * LAMPORTS_PER_SOL * 150_000_000 * TOKEN_MULT) * 100:.4f}%")
    print(f"  Total swaps: {pool.total_swaps}")
    print(f"  Note: LP value includes impermanent loss from price movement.")
    print(f"  Fee income grows K; IL may exceed fees on short timeframes with unbalanced flow.")
    assert pool.k > 200 * LAMPORTS_PER_SOL * 150_000_000 * TOKEN_MULT, "K must increase from fees"
    print("  ✓ K increased (fees compounding despite IL)")


def scenario_deep_vs_shallow():
    """Compare manipulation cost: shallow vs deep pool."""
    print("\n" + "="*50)
    print("  SCENARIO: Deep vs Shallow Pool")
    print("="*50)

    pools = {
        "Shallow (10 SOL)": DeepPool(10 * LAMPORTS_PER_SOL, 7_500_000 * TOKEN_MULT),
        "Medium (200 SOL)": DeepPool(200 * LAMPORTS_PER_SOL, 150_000_000 * TOKEN_MULT),
        "Deep (1000 SOL)": DeepPool(1000 * LAMPORTS_PER_SOL, 750_000_000 * TOKEN_MULT),
    }

    # Same 5 SOL buy in each
    attack = 5 * LAMPORTS_PER_SOL
    for name, pool in pools.items():
        price_before = pool.price
        tokens = pool.swap_buy(attack)
        price_after = pool.price
        impact = (price_after - price_before) / price_before * 100

        # Sell back
        sol_back = pool.swap_sell(tokens)
        loss = attack - sol_back

        print(f"\n  {name}:")
        print(f"    Price impact: {impact:.2f}%")
        print(f"    Net loss: {loss / LAMPORTS_PER_SOL:.4f} SOL")


def scenario_torch_integration():
    """Simulate Torch migration + community LP + trading."""
    print("\n" + "="*50)
    print("  SCENARIO: Torch Integration")
    print("="*50)

    # Torch migrates: 200 SOL + 150M tokens, burns LP
    pool = DeepPool(200 * LAMPORTS_PER_SOL, 150_000_000 * TOKEN_MULT)
    burned_lp = pool.lp_supply  # Torch burns all initial LP
    pool.print_state("After Torch migration (LP burned)")

    # Community adds liquidity
    sol_added, community_lp = pool.add_liquidity(30_000_000 * TOKEN_MULT)
    print(f"\n  Community added: {sol_added / LAMPORTS_PER_SOL:.4f} SOL + 30M tokens")
    print(f"  Community LP: {community_lp:,}")
    print(f"  Burned LP (Torch): {burned_lp:,}")
    print(f"  Total LP supply: {pool.lp_supply:,}")

    # 2000 swaps
    rng = random.Random(789)
    for _ in range(2000):
        if rng.random() < 0.5:
            pool.swap_buy(rng.randint(1, 10) * LAMPORTS_PER_SOL // 10)
        else:
            pool.swap_sell(rng.randint(1, 1000) * 1000 * TOKEN_MULT)

    pool.print_state("After 2000 swaps")

    # Community redeems their LP
    sol_out, tokens_out = pool.remove_liquidity(community_lp)
    print(f"\n  Community redeemed:")
    print(f"    SOL: {sol_out / LAMPORTS_PER_SOL:.4f} (put in {sol_added / LAMPORTS_PER_SOL:.4f})")
    print(f"    Tokens: {tokens_out / TOKEN_MULT:,.0f} (put in 30,000,000)")
    sol_profit = sol_out - sol_added
    print(f"    SOL profit: {sol_profit / LAMPORTS_PER_SOL:.4f}")
    if sol_profit > 0:
        print("    ✓ Community LP earned from fees")

    # Torch's burned LP is still in the pool — permanent depth
    print(f"\n  Remaining pool (Torch's permanent liquidity):")
    pool.print_state("After community exit")


# ============================================================================
# Main
# ============================================================================

if __name__ == "__main__":
    print("DeepPool Economic Simulator v0.1")
    print("=" * 50)

    scenario_fee_compounding()
    scenario_manipulation_cost()
    scenario_lp_appreciation()
    scenario_deep_vs_shallow()
    scenario_torch_integration()

    print("\n\n" + "=" * 50)
    print("  ALL SCENARIOS COMPLETE")
    print("=" * 50)
