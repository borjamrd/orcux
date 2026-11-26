import type { Cart } from "./types";

const FREE_SHIPPING = 50;

// The total of a cart, in euros, shipping included.
export function total(cart: Cart): number {
  const items = cart.items.reduce((sum, item) => sum + item.price * item.qty, 0);
  const shipping = items >= FREE_SHIPPING ? 0 : 4.99;
  return Math.round((items + shipping) * 100) / 100;
}
