It does not build. The compiler says:

```
error[E0594]: cannot assign to `heads`, as it is a captured variable in a `Fn` closure
   --> /area/area.rs:232:33
    |
207 |                 let mut heads = 1000.0;
    |                     --------- `heads` declared here, outside the closure
208 |                 let stalks = scatter(p, 1.6, self.seed ^ 0x4ee, |q, s| {
    |                              -------                            ------ in this closure
    |                              |
    |                              expects `Fn` instead of `FnMut`
...
232 |                                 heads = hd;
    |                                 ^^^^^^^^^^ cannot assign

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0594`.

```

Answer with the whole of `area.rs` again, fixed, in one ```rust block.