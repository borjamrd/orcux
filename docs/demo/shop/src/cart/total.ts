import type { Cart } from "./types";

// The total of a cart, in euros.
export function total(cart: Cart): number {
  return cart.items.reduce((sum, item) => sum + item.price, 0);
}
