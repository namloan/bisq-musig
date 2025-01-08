# Adaptor signatures with Musig2

This article is tailored to 2of2 Multisig and how to use
the [library secp256k1-zkp](https://github.com/BlockstreamResearch/secp256k1-zkp/tree/master) for adaptor signatures.
API calls taken
from https://github.com/BlockstreamResearch/secp256k1-zkp/blob/master/src/modules/musig/musig.md#atomic-swaps
An adaptor signature is a construction, where one party must reveal a secret when it wants to use a signature prepared
by another party.
This explanation is tailored towards 2of2 scriptless Multisig. Generalising to n-of-n should be straight forward, if
needed.
Let's assume Bob wants Alice to reveal the secret $t$. Bob (and Alice) will create a signature for a payout transaction
of a 2of2 multisig such that when Alice uses it, she must reveal $t$ to Bob.

Let

$$T=t \cdot G$$

where \
$t$ is secret adaptor, the secret which will be revealed\
$T$ is public adaptor\
$G$ is the Generator point of secp256k1\
$m$ (the message) is the serialisation of a transaction which spends the output of the 2of2 Multisig.

The function $H_{tagged}(x_1,...,x_n)$ is a hash function where the name 'tagged' is used as literal to init the hash
and if $x_i$ is a Curvepoint then we use the compressed x-key instead. Operator '||' stands for concatenation.

$$H_{tagged}(x_1,...,x_n):=sha256(sha256(sha256('tagged')||sha256('tagged'))||x_1||...||x_n)$$

## key aggregation

Technically the key aggregation is of course not part of the signing process. However, for creating the signature using
MuSig2, the aggregated key must be constructed in this way.

$P_a = p_a \cdot G$ is Alice Public Key
generated with `secp256k1_keypair_create` and obtained with `secp256k1_keypair_pub`. The $P_a$ is send to Bob.

Alice does the Key Aggregation in MuSig2

$$(1) \hspace{5pt} P = a_a \cdot P_a + a_b \cdot P_b$$

$ \begin{eqnarray}
where \\ \hspace{5pt} a_a &=& H_{agg}(sha256(P_a,P_b),P_a) \\
a_b &=& 1 \\
P_a &\ne& P_b
\end{eqnarray}$\
by calling `secp256k1_musig_pubkey_agg` with the pubkeys of all participants.

### Sign round 1

Alice creates 2 Nonces:
$R_{a,1}$ and $R_{a,2}$ and sends them around. She keeps the secret nonce $r_{a,1}$ and $r_{a,2}$ for herself.
This is done via `secp256k1_musig_nonce_gen` and sending the public nonce to Bob.

Up to here, its independent of the message and can be precalculated.

### Sign round 2

From collected Nonce the aggregated Nonce $R$ is calculated:

$$(2)\hspace{5pt} R_1 = R_{a,1} + R_{b,1}$$

$$R_2 = R_{a,2} + R_{b,2}$$

$$b = H_{non}(R_1 , R_2, P, m) $$

$$(3)\hspace{5pt} R = R_1 + b \cdot R_2 + T$$

Note that the $T$ is added to $R$, this seperates normal MuSig2 from adaptive MuSig2.
This is done via `secp256k1_musig_nonce_agg` and `secp256k1_musig_nonce_process(T)`. This is split into 2 methods, in
other scenarios there could be one party aggregating the nounces and sending them around, which is better than everyone
sending to everyone.

Alice creates her partial signature

$$(4)\hspace{5pt} s_a = r_{a,1} + b \cdot r_{a,2} + a_a \cdot H_{sig}(R,P,m) \cdot p_a$$

$s_a$ is the partial signature calculated via `secp256k1_musig_partial_sign` (Note that the public Nonces have been
exchanged already)

Note, that when Bob calculates his partial signature with (4) he can be sure, that his signature can only be used with
the adaptor $T$, since $R$ is part of the hash-function and $R$ is dependent of $T$.
The partial signature get exchanged. And the other side can verify the validity of the partial signature.
We can multiply equation (4) with $G$ and verify the partial signature with this form:

$$s_a \cdot G = R_{a,1} + b \cdot R_{a,2}+a_a \cdot H_{sig}(R,P,m) \cdot P_a$$

This is being done by calling
`secp256k1_musig_partial_sig_verify`.

## Signature aggregation

The partial signatures get aggregated using

$$s=s_a+s_b$$

with the method `secp256k1_musig_partial_sig_agg`. Since we added a $T$ into the signature (3), this is only a
pre-signature, not a valid signature.
We can prove that this pre-signature is indeed a valid adaptor signature by multiplying with $G$ and
setting $e:=H_{sig}(R,P,m)$:

$\begin{eqnarray}
s\cdot G &=& s_a\cdot G+s_b\cdot G ;| \hspace{3pt} with (4) \\
&=&(r_{a,1} + b \cdot r_{a,2} + a_a \cdot e \cdot p_a)\cdot G + (r_{b,1}+b \cdot r_{b,2} + a_b \cdot e \cdot p_b)\cdot G \\
&=&R_{a,1}+b\cdot R_{a,2} + a_a\cdot e \cdot P_a + R_{b,1}+b\cdot R_{b,2} + a_b\cdot e \cdot P_b;| \hspace{3pt} with (2) \\
&=& R_1 + b \cdot R_2 + a_a \cdot e \cdot P_a + a_b \cdot e \cdot P_b;| \hspace{3pt} with (1) \\
&=& R_1 + b \cdot R_2 + e \cdot P;| \hspace{3pt} with (3) \\
&=& R - T + e \cdot P
\end{eqnarray}$

that means

$$\begin{eqnarray}
s \cdot G + T &=& R+e \cdot P \\
\Leftrightarrow (s + t) \cdot G &=& R + e \cdot P
\end{eqnarray}$$

so with the discrete logarithm (DLOG) of $T$, which is  $t$, we would have a valid signature.

All parties involved extract this session's `nonce_parity` with `secp256k1_musig_nonce_parity`. This is needed because
of some internal implementation details, namely how we handle curve point, as compressed Pubkey or x-only Pubkey. Alice
has the knowledge of $t$ and can make this a valid signature.
Alice must "adapt" the pre-signature with `t` and the `nonce_parity` using  `secp256k1_musig_adapt` and
gets  $s' = s+t$.

Alice can use the signature $Sig(s',R)$ and broadcast the Transaction. In the mempool or on the blockchain, Bob can
find $s'$. Bob can calculate $t=s' - s$ by using `secp256k1_musig_extract_adaptor` with $s, s'$ and `nonce_parity`.
