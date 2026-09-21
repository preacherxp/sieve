package example;
public class PriceCalculator { public int total(int net) { return net + net * new TaxRules().percent() / 100; } }
