package example;
public class CardGateway implements PaymentGateway { public String charge(int amount) { return "card:" + amount; } }
