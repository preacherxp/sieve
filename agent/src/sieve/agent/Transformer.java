package sieve.agent;

import java.lang.instrument.ClassFileTransformer;
import java.net.URL;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.CodeSource;
import java.security.ProtectionDomain;
import java.util.List;
import java.util.Set;
import sieve.agent.asm.ClassReader;
import sieve.agent.asm.ClassVisitor;
import sieve.agent.asm.ClassWriter;
import sieve.agent.asm.MethodVisitor;
import sieve.agent.asm.Opcodes;

/**
 * Adds a method-entry probe to every method of the project's own classes, and a file probe to
 * the JDK constructors and methods that open files. Uses the vendored ASM (ADR 0003), so it
 * runs on Java 17+; a class file ASM cannot read fails here and keeps the run's records out.
 */
final class Transformer implements ClassFileTransformer {
    private static final String PROBE = "sieve/probe/Probe";
    private static final String HIT = "(I)V";
    private static final String FILE = "(Ljava/lang/Object;)V";
    private static final Set<String> LISTINGS = Set.of("list", "listFiles", "newDirectoryStream");
    /** File-system provider methods whose first parameter is the path they open, list, or probe. */
    private static final Set<String> FILE_METHODS = Set.of("newByteChannel", "newFileChannel", "newAsynchronousFileChannel",
            "newDirectoryStream", "copy", "checkAccess", "readAttributes", "readAttributesIfExists", "exists",
            "isDirectory", "isRegularFile");
    /** {@code java.io.File} methods that look at the file they are called on. */
    private static final Set<String> SELF_METHODS = Set.of("exists", "isFile", "isDirectory", "length", "list", "listFiles");

    private final Set<Path> outputs = new java.util.HashSet<>();
    private final Set<String> jdk;
    private final java.util.Map<String, java.util.Optional<Path>> locations = new java.util.concurrent.ConcurrentHashMap<>();

    Transformer(List<Path> outputs, Set<String> jdk) {
        // Real paths, so that a symlinked or differently spelled checkout still matches.
        for (Path output : outputs) {
            this.outputs.add(real(output));
        }
        this.jdk = Set.copyOf(jdk);
    }

    private static Path real(Path path) {
        try {
            return path.toRealPath();
        } catch (java.io.IOException error) {
            return path.toAbsolutePath().normalize();
        }
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined, ProtectionDomain domain, byte[] bytes) {
        if (name == null) {
            return null;
        }
        try {
            if (jdk.contains(name)) {
                return fileProbes(bytes);
            }
            Path dir = output(domain);
            // Proxies defined into an output directory's domain have no class file there.
            if (dir == null || !Files.isRegularFile(dir.resolve(name + ".class"))) {
                return null;
            }
            byte[] probed = methodProbes(name, bytes);
            State.instrumented();
            return probed;
        } catch (Throwable error) {
            State.error(name + ": " + error);
            return null;
        }
    }

    private Path output(ProtectionDomain domain) throws Exception {
        CodeSource source = domain == null ? null : domain.getCodeSource();
        URL location = source == null ? null : source.getLocation();
        if (location == null || !"file".equals(location.getProtocol())) {
            return null;
        }
        return locations.computeIfAbsent(location.toString(), key -> {
            try {
                Path path = real(Path.of(location.toURI()));
                return java.util.Optional.ofNullable(outputs.contains(path) ? path : null);
            } catch (java.net.URISyntaxException | RuntimeException error) {
                return java.util.Optional.empty();
            }
        }).orElse(null);
    }

    /**
     * Inserts {@code prologue} at the start of every method it returns a slot or id for. Code
     * is only prepended and leaves the operand stack empty, so existing stack map frames stay
     * valid and nothing is loaded to compute new ones; only the maximum stack is recomputed.
     */
    private static byte[] prepend(byte[] bytes, Prologue prologue) {
        ClassReader reader = new ClassReader(bytes);
        ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
        reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
            private String owner;

            @Override
            public void visit(int version, int access, String name, String signature, String superName, String[] interfaces) {
                owner = name;
                super.visit(version, access, name, signature, superName, interfaces);
            }

            @Override
            public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
                MethodVisitor next = super.visitMethod(access, name, descriptor, signature, exceptions);
                return new MethodVisitor(Opcodes.ASM9, next) {
                    @Override
                    public void visitCode() {
                        super.visitCode();
                        prologue.emit(this, owner, access, name, descriptor);
                    }
                };
            }
        }, 0);
        return writer.toByteArray();
    }

    private static byte[] methodProbes(String name, byte[] bytes) {
        return prepend(bytes, (code, owner, access, method, descriptor) -> {
            code.visitLdcInsn(State.method(name + "#" + method + descriptor));
            code.visitMethodInsn(Opcodes.INVOKESTATIC, PROBE, "hit", HIT, false);
        });
    }

    /**
     * Reports the {@code File}, {@code String}, or {@code Path} that a method opens, lists, or
     * probes: the first parameter, or the {@code File} itself.
     */
    private static byte[] fileProbes(byte[] bytes) {
        return prepend(bytes, (code, owner, access, method, descriptor) -> {
            int slot = slot(owner, access, method, descriptor);
            if (slot >= 0) {
                code.visitVarInsn(Opcodes.ALOAD, slot);
                code.visitMethodInsn(Opcodes.INVOKESTATIC, PROBE, LISTINGS.contains(method) ? "list" : "file", FILE, false);
            }
        });
    }

    /** The local variable slot holding the file a method uses, or -1. */
    private static int slot(String owner, int access, String name, String type) {
        if ((access & Opcodes.ACC_STATIC) != 0) {
            return -1;
        }
        if (owner.equals("java/io/File")) {
            return SELF_METHODS.contains(name) ? 0 : -1;
        }
        if (name.equals("<init>")) {
            return type.startsWith("(Ljava/io/File;") ? 1 : -1;
        }
        return FILE_METHODS.contains(name) && type.startsWith("(Ljava/nio/file/Path;") ? 1 : -1;
    }

    /** Code to insert at the start of a method with code. */
    private interface Prologue {
        void emit(MethodVisitor code, String owner, int access, String method, String descriptor);
    }
}
