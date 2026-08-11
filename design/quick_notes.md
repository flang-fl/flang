```
comp get = fn(&self, index: usize) -> Option(T) [
  case
    where index < self.length
    ensures return is Some(...) {
      return Some(self.arr[index]);
    }
  
  case -> Option(T)::None
    where index >= self.length {
      return None;
    }
    
  // same as
  case
    where index >= self.length
    ensures return is None {
      return None;
    } 
    
  // same as
  case(&self, index: usize) -> Option(T)
    where index >= self.length
    ensures return is None {
      return None;
    }
    
  // same as
  case where index >= self.length => None;
]
```

```
if x is 5 {

}

if x is Some(...) {

}

if x is Some(let y) {

}

x is Some(let y) else {
  return 5;
}

if x is > 5 {

}
or
if x is ? > 5 {

} but verbose
```

```
let balls = match [
  case 
    where x is 5
    ensures return is >= 5 {
      return x + 1;
    }
  
  case
    where x isnt 5
    ensures return is 5 {
      return x + 1;
    }
]
```
```
isnt as a !is is kind of sick.

is also doubles naturally as a type check but requires including some kind of
pattern matching to differentiate:

x is Type(...) or x is Type { .. }

vs

T is Type


this becomes interesting with empty types, what differentiates

x is None and T is None, is T is None even legal? or would it need to be
T is Option(T)::None because None is a generic so you cant ungeneric it that easily
optionally T is Option(...)::None or if it had multipl args
T is Option(T = ..., other args)::None

This also shows why _ might be better
T is Option(T = _, other args)::None is cleaner

if x isnt 5 {

}
is SICK as hell 
```

```
comp divide = fn(a: i32, b: i32) -> Option(i32) [
  case where b == 0 ensures return is None => None,
  default ensures return is Some(...) {
    return Some(a / b);
  }
]
```