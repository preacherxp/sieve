package example;

public record PriceQuote(int net, int total) {
    public PriceQuote {
        if (net < 0 || total < net) throw new IllegalArgumentException("Invalid price quote");
    }

    public static PriceQuote priced(int net) {
        return new PriceQuote(net, new PriceCalculator().total(net));
    }
}
