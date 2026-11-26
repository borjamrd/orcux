import { total } from "../cart/total";
import type { Cart } from "../cart/types";

export function Summary({ cart }: { cart: Cart }) {
  return (
    <section>
      {cart.items.map((i) => <p key={i.name}>{i.name}</p>)}
    </section>
  );
}
