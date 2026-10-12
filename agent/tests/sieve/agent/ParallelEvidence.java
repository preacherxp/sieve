package sieve.agent;

import java.nio.file.Path;
import java.util.Properties;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CyclicBarrier;
import sieve.probe.Bucket;
import sieve.probe.Probe;

/** Exercises real overlap, worker-thread reads, and a Spring context reused after overlap. */
public final class ParallelEvidence {
    public static final class Context {
        public Object getBeanFactory() { return this; }
    }

    public static void main(String[] args) throws Exception {
        Path workspace = Path.of(args[0]);
        State.configure(workspace, args[1], new Properties());
        Probe.roots(workspace);
        State.instrumented();
        Context first = new Context();
        Context second = new Context();
        CyclicBarrier barrier = new CyclicBarrier(2);
        State.classStarted("example.FirstTest");
        State.classStarted("example.FirstTest");
        State.classFinished("example.FirstTest");
        State.bootStarting();
        State.contextStarting(first);
        Probe.hit(State.method("example/First#startup()V"));
        Thread other = new Thread(() -> {
            try {
                State.classStarted("example.SecondTest");
                State.bootStarting();
                State.contextStarting(second);
                barrier.await(10, java.util.concurrent.TimeUnit.SECONDS);
                CompletableFuture.runAsync(() -> {
                    Probe.hit(State.method("example/Second#worker()V"));
                    Probe.file(workspace.resolve("async-input.txt"));
                }).join();
                State.usesContext("example.SecondTest", second);
                barrier.await(10, java.util.concurrent.TimeUnit.SECONDS);
                State.contextStarted(second);
                State.bootFinished();
                State.classFinished("example.SecondTest");
            } catch (Exception error) {
                throw new AssertionError(error);
            }
        });
        other.start();
        barrier.await(10, java.util.concurrent.TimeUnit.SECONDS);
        Probe.hit(State.method("example/First#test()V"));
        State.usesContext("example.FirstTest", first);
        State.contextStarted(first);
        State.bootFinished();
        barrier.await(10, java.util.concurrent.TimeUnit.SECONDS);
        other.join();
        State.classFinished("example.FirstTest");

        State.classStarted("example.CachedContextTest");
        State.usesContext("example.CachedContextTest", first);
        Bucket previous = Probe.singletonStarting(first);
        Probe.hit(State.method("example/Lazy#initialize()V"));
        Probe.singletonFinished(previous);
        State.classFinished("example.CachedContextTest");

        State.classStarted("example.IndependentTest");
        Probe.hit(State.method("example/Independent#test()V"));
        State.classFinished("example.IndependentTest");
        var json = State.class.getDeclaredMethod("json");
        json.setAccessible(true);
        String firstPlan = (String) json.invoke(null);
        State.flush();

        State.classStarted("example.RepeatedTest");
        Probe.hit(State.method("example/Repeated#first()V"));
        State.classStarted("example.PartnerTest");
        Probe.hit(State.method("example/Partner#test()V"));
        State.classFinished("example.PartnerTest");
        State.classFinished("example.RepeatedTest");
        State.flush();
        State.classStarted("example.RepeatedTest");
        Probe.hit(State.method("example/Repeated#second()V"));
        State.failed("example.RepeatedTest");
        State.classFinished("example.RepeatedTest");
        System.out.print("[" + firstPlan + "," + json.invoke(null) + "]");
    }
}
